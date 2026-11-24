//! 插件包：文件夹（根目录有 plugin.json）或 `.mpx`（就是 zip）。解压检查、整包摘要、复制到插件目录
use super::manifest::Manifest;
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// 解压后最多这么大
pub const MAX_UNPACKED: u64 = 200 << 20;
pub const MAX_ENTRIES: usize = 5000;

/// 打开的插件包
pub struct Opened {
    /// 插件文件夹（plugin.json 所在）
    pub dir: PathBuf,
    /// 解压出来的临时目录，用完删掉
    pub temp: Option<PathBuf>,
    pub manifest: Manifest,
    pub text: String,
}

impl Drop for Opened {
    fn drop(&mut self) {
        if let Some(t) = &self.temp {
            let _ = std::fs::remove_dir_all(t);
        }
    }
}

pub fn is_zip(path: &Path) -> bool {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut sig = [0u8; 4];
    f.read_exact(&mut sig).is_ok() && sig == *b"PK\x03\x04"
}

/// 打开文件夹或 .mpx。`staging` 是解压用的目录（在插件目录下，安装时可以直接改名过去）
pub fn open(path: &Path, staging: &Path) -> Result<Opened> {
    if path.is_dir() {
        let dir = find_root(path)?;
        let (manifest, text) = read_manifest(&dir)?;
        return Ok(Opened {
            dir,
            temp: None,
            manifest,
            text,
        });
    }
    if !is_zip(path) {
        bail!("不是插件包：要选插件文件夹（里面有 plugin.json）或 .mpx 文件");
    }
    std::fs::create_dir_all(staging)?;
    let tmp = staging.join(format!("unpack-{}-{}", std::process::id(), nanos()));
    unzip(path, &tmp).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&tmp);
    })?;
    let opened = (|| {
        let dir = find_root(&tmp)?;
        let (manifest, text) = read_manifest(&dir)?;
        Ok::<_, anyhow::Error>((dir, manifest, text))
    })();
    match opened {
        Ok((dir, manifest, text)) => Ok(Opened {
            dir,
            temp: Some(tmp),
            manifest,
            text,
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// plugin.json 在根目录，或者在唯一的一层子文件夹里（直接压缩文件夹时常见）
fn find_root(dir: &Path) -> Result<PathBuf> {
    if dir.join("plugin.json").is_file() {
        return Ok(dir.to_path_buf());
    }
    let subs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("__MACOSX")))
        .collect();
    if subs.len() == 1 && subs[0].join("plugin.json").is_file() {
        return Ok(subs[0].clone());
    }
    bail!("插件包里没有 plugin.json")
}

fn read_manifest(dir: &Path) -> Result<(Manifest, String)> {
    let bytes = std::fs::read(dir.join("plugin.json")).context("读不到 plugin.json")?;
    if bytes.len() > 256 * 1024 {
        bail!("plugin.json 太大");
    }
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("plugin.json 要用 UTF-8 编码"))?;
    let m = Manifest::parse(&text).map_err(|e| anyhow!(e))?;
    Ok((m, text))
}

/// 安全解压：不允许绝对路径、`..`、符号链接，限制总大小和文件数
pub fn unzip(zip_path: &Path, to: &Path) -> Result<()> {
    let f = std::fs::File::open(zip_path)?;
    let mut z = zip::ZipArchive::new(f).context("插件包不是有效的 zip")?;
    if z.len() > MAX_ENTRIES {
        bail!("插件包里的文件太多（上限 {MAX_ENTRIES} 个）");
    }
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let name = e.name().replace('\\', "/");
        let rel = Path::new(&name);
        if name.starts_with('/')
            || rel
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir))
        {
            bail!("插件包里有不安全的路径：{name}");
        }
        if e.is_symlink() {
            bail!("插件包里不能有符号链接：{name}");
        }
        let out = to.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        total += e.size();
        if total > MAX_UNPACKED {
            bail!("插件包解压后太大（上限 {} MB）", MAX_UNPACKED >> 20);
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        // 实际写入量也要限制（zip 里声明的大小可能是假的）
        let n = std::io::copy(&mut (&mut e).take(MAX_UNPACKED + 1), &mut w)?;
        if n > e.size() {
            bail!("插件包里的文件大小和声明的不一致：{name}");
        }
    }
    Ok(())
}

/// 整个文件夹的摘要：按相对路径排序，路径和内容一起算（改了任何文件都会变）
pub fn digest(dir: &Path) -> Result<String> {
    let mut files = vec![];
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for rel in files {
        h.update(rel.as_bytes());
        h.update([0]);
        let mut f = std::fs::File::open(dir.join(&rel))?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        h.update([0]);
    }
    Ok(hex::encode(h.finalize()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let ft = e.file_type()?;
        if ft.is_dir() {
            // 插件自己的缓存目录、Python 的编译缓存不算
            let n = e.file_name().to_string_lossy().into_owned();
            if n == "__pycache__" || n == "node_modules" || n == ".git" {
                continue;
            }
            collect(root, &p, out)?;
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// 复制文件夹（安装文件夹形式的插件时用）
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let ft = e.file_type()?;
        let dst = to.join(e.file_name());
        if ft.is_dir() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == ".git" || n == "node_modules" || n == "__pycache__" {
                continue;
            }
            copy_dir(&e.path(), &dst)?;
        } else if ft.is_file() {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// 把文件夹打成 .mpx（开发者模式“导出插件包”和测试用）
pub fn pack(dir: &Path, out: &Path) -> Result<()> {
    let mut files = vec![];
    collect(dir, dir, &mut files)?;
    files.sort();
    let f = std::fs::File::create(out)?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for rel in files {
        z.start_file(rel.as_str(), opt)?;
        let mut src = std::fs::File::open(dir.join(&rel))?;
        std::io::copy(&mut src, &mut z)?;
    }
    z.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mp-plg-pkg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const MANIFEST: &str = r#"{"manifest":1,"id":"demo","name":"Demo","version":"1.0.0"}"#;

    #[test]
    fn folder_zip_and_nested_zip() {
        let d = tmp("ok");
        let src = d.join("src");
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("plugin.json"), MANIFEST).unwrap();
        std::fs::write(src.join("lib/a.txt"), "x").unwrap();
        let o = open(&src, &d.join("stage")).unwrap();
        assert_eq!(o.manifest.id, "demo");
        assert!(o.temp.is_none());
        let h1 = digest(&src).unwrap();

        let mpx = d.join("demo.mpx");
        pack(&src, &mpx).unwrap();
        let o = open(&mpx, &d.join("stage")).unwrap();
        assert!(o.dir.join("lib/a.txt").is_file());
        assert_eq!(digest(&o.dir).unwrap(), h1, "打包再解压，摘要不变");
        let t = o.temp.clone().unwrap();
        drop(o);
        assert!(!t.exists(), "临时目录用完删掉");

        // 压缩时多套了一层文件夹
        let nested = d.join("nested.mpx");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&nested).unwrap());
        let opt = zip::write::SimpleFileOptions::default();
        z.start_file("demo/plugin.json", opt).unwrap();
        z.write_all(MANIFEST.as_bytes()).unwrap();
        z.finish().unwrap();
        assert_eq!(open(&nested, &d.join("stage")).unwrap().manifest.name, "Demo");

        std::fs::write(src.join("lib/a.txt"), "y").unwrap();
        assert_ne!(digest(&src).unwrap(), h1, "改了文件摘要会变");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_unsafe_zips() {
        let d = tmp("bad");
        let bad = d.join("bad.mpx");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        let opt = zip::write::SimpleFileOptions::default();
        z.start_file("../evil.txt", opt).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        let e = open(&bad, &d.join("stage")).err().unwrap().to_string();
        assert!(e.contains("不安全"), "{e}");
        assert!(!d.join("evil.txt").exists());

        std::fs::write(d.join("x.txt"), "hello").unwrap();
        assert!(open(&d.join("x.txt"), &d.join("stage")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
