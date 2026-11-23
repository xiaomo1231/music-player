use super::export;
use super::img::{self, Half};
use super::spec::{self, Spec};
use super::view::{self, View};
use crate::em::format;
use image::{DynamicImage, Rgb, RgbImage};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const HEAD: &str = "em: 1\nkind: comic\nid: t-comic\nname: 测试\nversion: 1.2.0\n";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mp-em-comic-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn compile(body: &str) -> Result<Spec, Vec<crate::em::Issue>> {
    let rule = format::parse(&format!("{HEAD}{body}"))?;
    spec::compile(&rule)
}

fn spec(body: &str) -> Arc<Spec> {
    Arc::new(compile(body).unwrap_or_else(|e| panic!("{e:?}")))
}

/// 单色页面，左右两半颜色不同（拆开后能分辨）
fn page(w: u32, h: u32, left: [u8; 3], right: [u8; 3]) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, _| Rgb(if x < w / 2 { left } else { right })))
}

fn png(im: &DynamicImage) -> Vec<u8> {
    img::encode(im, img::OutFmt::Png, 90).unwrap()
}

fn make_zip(path: &Path, pages: &[(&str, Vec<u8>)]) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    for (n, b) in pages {
        z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
}

fn build(path: &Path, s: Arc<Spec>) -> View {
    view::build(path, "t-comic", s, "v".into()).unwrap()
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GRAY: [u8; 3] = [90, 90, 90];

fn color_of(bytes: &[u8]) -> [u8; 3] {
    let (im, _) = img::decode(bytes).unwrap();
    let rgb = im.to_rgb8();
    let p = rgb.get_pixel(rgb.width() / 2, rgb.height() / 2);
    p.0
}

fn book(dir: &Path) -> PathBuf {
    // p1 封面（宽）、p2 普通、p3 跨页（左红右蓝）、p10 普通
    let z = dir.join("vol.cbz");
    make_zip(
        &z,
        &[
            ("p10.png", png(&page(100, 140, GRAY, GRAY))),
            ("p1.png", png(&page(300, 140, RED, BLUE))),
            ("p3.png", png(&page(300, 140, RED, BLUE))),
            ("p2.png", png(&page(100, 140, GRAY, GRAY))),
        ],
    );
    z
}

#[test]
fn split_rtl_ltr_and_cover() {
    let d = tmp("split");
    let z = book(&d);
    let v = build(&z, spec("pages:\n  split_wide: { when_ratio: 1.3, order: rtl, keep_cover: true }\n"));
    let names: Vec<&str> = v.pages.iter().map(|p| view::base_name(&v.names[p.src])).collect();
    assert_eq!(names, vec!["p1.png", "p2.png", "p3.png", "p3.png", "p10.png"]);
    assert_eq!(v.pages[0].half, Half::Full, "封面不拆");
    assert_eq!((v.pages[2].half, v.pages[3].half), (Half::Right, Half::Left));
    assert_eq!(color_of(&v.render(2).unwrap().0), BLUE, "从右往左：先右半");
    assert_eq!(color_of(&v.render(3).unwrap().0), RED);

    let v = build(&z, spec("pages:\n  split_wide: { order: ltr, keep_cover: false }\n"));
    assert_eq!(v.len(), 6);
    assert_eq!((v.pages[0].half, v.pages[1].half), (Half::Left, Half::Right));
    assert_eq!(color_of(&v.render(0).unwrap().0), RED);
    // 跨页拆开默认跟着阅读方向
    let v = build(&z, spec("read: { dir: rtl }\npages:\n  split_wide: true\n"));
    assert_eq!(v.pages[2].half, Half::Right);
}

#[test]
fn skip_name_last_and_like() {
    let d = tmp("skip");
    let credit = page(120, 160, [250, 250, 250], [20, 20, 20]);
    // 样张放在规则文件夹里
    let sample = format!("sample-{}.png", std::process::id());
    std::fs::create_dir_all(crate::em::rules_dir()).unwrap();
    std::fs::write(crate::em::rules_dir().join(&sample), png(&credit)).unwrap();
    let mut grad = RgbImage::new(100, 140);
    for (x, y, p) in grad.enumerate_pixels_mut() {
        *p = Rgb([(y * 2) as u8, (x * 2) as u8, 60]);
    }
    let z = d.join("c.cbz");
    make_zip(
        &z,
        &[
            ("01.png", png(&page(100, 140, GRAY, GRAY))),
            ("02.png", png(&DynamicImage::ImageRgb8(grad))),
            ("03_汉化组招募.png", png(&page(100, 140, GRAY, GRAY))),
            ("04.png", png(&credit.resize_exact(100, 133, image::imageops::FilterType::Triangle))),
            ("05.png", png(&page(100, 140, GRAY, GRAY))),
            ("06.png", png(&page(100, 140, GRAY, GRAY))),
        ],
    );
    let v = build(
        &z,
        spec(&format!(
            "pages:\n  skip:\n    - name: '(?i)(credit|招募)'\n    - last: 1\n    - like: {sample}\n      distance: 3\n"
        )),
    );
    let kept: Vec<&str> = v.pages.iter().map(|p| v.names[p.src].as_str()).collect();
    assert_eq!(kept, vec!["01.png", "02.png", "05.png"]);
    let why: Vec<(usize, &str)> = v.skipped.iter().map(|s| (s.src, s.why.as_str())).collect();
    assert_eq!(why[0], (2, "文件名"));
    assert!(why[1].1.starts_with("像"));
    assert_eq!(why[2], (5, "结尾"));
    let _ = std::fs::remove_file(crate::em::rules_dir().join(&sample));

    // 样张不存在：只提示，不报错
    let s = compile("pages:\n  skip:\n    - like: not-here.png\n").unwrap();
    assert!(s.summary.iter().any(|l| l.contains("not-here.png") && l.contains("规则文件夹")));
}

#[test]
fn trim_never_over_crops() {
    let d = tmp("trim");
    // 白边 20px，中间是贴着左边的深色画面
    let mut im = RgbImage::from_pixel(200, 200, Rgb([255, 255, 255]));
    for y in 20..180 {
        for x in 0..150 {
            im.put_pixel(x, y, Rgb([15, 15, 15]));
        }
    }
    let z = d.join("t.cbz");
    make_zip(&z, &[("1.png", png(&DynamicImage::ImageRgb8(im)))]);
    let v = build(&z, spec("pages:\n  trim: { color: auto, tolerance: 12, max: 5% }\n"));
    let (b, mime) = v.render(0).unwrap();
    assert_eq!(mime, "image/png", "PNG 原图处理后还是 PNG");
    let (out, _) = img::decode(&b).unwrap();
    // 上下各 20px 白边只能裁到 10px（5%），左边贴着画面不裁，右边 50px 白边裁到 10px
    assert_eq!((out.width(), out.height()), (190, 180));
    assert!(v.trimmed(0));
}

fn read_zip(p: &Path) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(p).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = vec![];
            f.read_to_end(&mut b).unwrap();
            (f.name().to_string(), b)
        })
        .collect()
}

#[test]
fn export_order_marker_and_no_double_processing() {
    let d = tmp("export");
    let z = book(&d);
    let body =
        "read: { dir: rtl }\npages:\n  split_wide: { when_ratio: 1.3, keep_cover: true }\nexport:\n  format: jpg\n  quality: 80\n  rename: '{序号:03}.{ext}'\n";
    let s = spec(body);
    let v = build(&z, s.clone());
    let meta = export::Meta {
        rule_id: "t-comic",
        rule_version: "1.2.0",
        title: "第1卷",
        series: "测试<作品>",
    };
    let out = export::target_for(&z);
    assert!(out.to_string_lossy().ends_with("vol [排版].cbz"));
    let mut seen = 0;
    let n = export::write(&v, &s, &meta, &out, |done, _| seen = done).unwrap();
    assert_eq!((n, seen), (5, 5));
    let files = read_zip(&out);
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, vec!["001.jpg", "002.jpg", "003.jpg", "004.jpg", "005.jpg", "ComicInfo.xml"]);
    // 先右半（蓝）再左半（红）
    let c3 = color_of(&files[2].1);
    let c4 = color_of(&files[3].1);
    assert!(c3[2] > 200 && c3[0] < 60, "{c3:?}");
    assert!(c4[0] > 200 && c4[2] < 60, "{c4:?}");
    let xml = String::from_utf8(files[5].1.clone()).unwrap();
    assert!(xml.contains("<Notes>em:t-comic@1.2.0</Notes>"));
    assert!(xml.contains("<Manga>YesAndRightToLeft</Manga>"));
    assert!(xml.contains("<Series>测试&lt;作品&gt;</Series>"));
    assert!(xml.contains("<PageCount>5</PageCount>"));

    // 导出的文件再打开：认出标记，不再拆页
    let again = build(&out, s.clone());
    assert!(again.marker && !again.processed);
    assert_eq!(again.len(), 5);
    assert!(view::has_marker(&xml, "t-comic"));
    assert!(!view::has_marker(&xml, "t-com"));
    // 第二次导出不会覆盖：换个名字
    assert!(export::target_for(&z).to_string_lossy().ends_with("vol [排版] (2).cbz"));
}

#[test]
fn export_slices_long_strip() {
    let d = tmp("slice");
    // 宽 60、高 3000：画面块之间留 30px 白
    let mut im = RgbImage::from_pixel(60, 3000, Rgb([255, 255, 255]));
    let art = [(0u32, 900u32), (930, 1900), (1930, 3000)];
    for &(a, b) in &art {
        for y in a..b {
            for x in 0..60 {
                im.put_pixel(x, y, Rgb([((x * 5 + y) % 200) as u8, 40, 90]));
            }
        }
    }
    let z = d.join("strip.cbz");
    make_zip(&z, &[("a.png", png(&DynamicImage::ImageRgb8(im)))]);
    let s = spec("export:\n  slice: { max_height: 1200, at_blank_rows: true }\n  format: png\n");
    let v = build(&z, s.clone());
    assert!(!v.processed, "只有导出选项时阅读不处理");
    let meta = export::Meta {
        rule_id: "t-comic",
        rule_version: "",
        title: "",
        series: "",
    };
    let out = d.join("o.cbz");
    export::write(&v, &s, &meta, &out, |_, _| {}).unwrap();
    let files = read_zip(&out);
    let heights: Vec<u32> = files
        .iter()
        .filter(|f| f.0.ends_with(".png"))
        .map(|f| img::decode(&f.1).unwrap().0.height())
        .collect();
    assert_eq!(heights.iter().sum::<u32>(), 3000);
    assert!(heights.len() >= 3);
    let mut y = 0;
    for h in &heights[..heights.len() - 1] {
        y += h;
        assert!(art.iter().all(|&(a, b)| !(a < y && y < b)), "切到了画面：{y}");
    }
    assert!(String::from_utf8_lossy(&files.last().unwrap().1).contains("<Notes>em:t-comic</Notes>"));
}

#[test]
fn validation_errors_have_lines() {
    let e = compile("match:\n  - series: '(未闭合'\npages:\n  trim: { max: 80% }\n").unwrap_err();
    assert_eq!(e[0].line, 7, "{e:?}");
    assert!(e[0].msg.contains("正则"));
    assert_eq!(e[1].line, 9);
    assert!(e[1].msg.contains("max"));

    let e = compile("export:\n  quality: 90\n  format: webp\n").unwrap_err();
    assert_eq!(e[0].line, 8);
    assert!(e[0].msg.contains("暂不支持导出 webp"));

    let e = compile("read:\n  flow: zigzag\n").unwrap_err();
    assert_eq!(e[0].line, 7);
    let e = compile("pages:\n  spilt_wide: true\n").unwrap_err();
    assert!(e[0].msg.contains("spilt_wide"));
    let e = compile("export:\n  rename: 'page.{ext}'\n").unwrap_err();
    assert!(e[0].msg.contains("{序号}"));
    let e = compile("layout: 1\n").unwrap_err();
    assert_eq!(e[0].line, 6);
}

#[test]
fn summary_lines() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/em-examples/manga-layout.em")).unwrap();
    let rule = format::parse(&text).unwrap();
    let s = spec::compile(&rule).unwrap();
    let all = s.summary.join("\n");
    assert!(all.contains("用于：作品名匹配 .*"), "{all}");
    assert!(all.contains("阅读：左右翻页、从右往左、双页、封面单独"));
    assert!(all.contains("拆开宽高比超过 1.3 的跨页"));
    assert!(all.contains("每边最多 8%"));
    assert!(all.contains("每话最后 1 页"));
    assert!(all.contains("导出：JPG 90、最大高度 3000"));
    assert!(s.matches("任意", "D:\\x.cbz"));

    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/em-examples/webtoon-strip.em")).unwrap();
    let s = spec::compile(&format::parse(&text).unwrap()).unwrap();
    assert!(s.matches("", "D:\\漫画\\条漫\\a.cbz"));
    assert!(s.matches("", "/mnt/漫画/条漫/a.cbz"), "斜杠方向不同也能匹配");
    assert!(!s.matches("", "D:\\漫画\\日漫\\a.cbz"));
    assert!(s.summary.iter().any(|l| l.contains("只裁左右")));
}

#[test]
fn rename_templates() {
    let p = spec::parse_rename("{序号:03}.{ext}").unwrap();
    assert_eq!(spec::render_name(&p, 7, "jpg", "x"), "007.jpg");
    let p = spec::parse_rename("{原名}_{n}.{ext}").unwrap();
    assert_eq!(spec::render_name(&p, 12, "png", "a"), "a_12.png");
    assert!(spec::parse_rename("{序号:3}.{ext}").is_err());
    assert!(spec::parse_rename("x/{序号}.{ext}").is_err());
}
