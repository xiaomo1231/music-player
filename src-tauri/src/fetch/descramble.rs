//! 漫画图片还原：有些站点把一页图切成若干横带打乱顺序，网页上用 canvas 现场拼回。
//! 下载源在 `Target::Pages` 里用 `descramble` 标明算法，下载每页后在这里拼回再写盘。
//! 目前只认 `jm18comic`（18comic）。

use image::{DynamicImage, GenericImage, GenericImageView, ImageFormat};
use md5::{Digest, Md5};
use std::io::Cursor;

/// 18comic：从这个相册号起图片才被打乱，更早的相册是正常顺序
const JM_SCRAMBLE_FROM: u64 = 220_981;

/// 按 `descramble` 标记还原一页图片的字节。不认识的算法、不需要还原或解码失败时返回 `None`（原样保存）
pub fn fix_page(kind: &str, url: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    match kind {
        "jm18comic" => {
            let (aid, page) = jm_ids_from_url(url)?;
            jm_fix_bytes(bytes, aid, &page)
        }
        _ => None,
    }
}

/// 从图片地址取相册号和页名：`…/media/photos/1222566/00002.webp` → (1222566, "00002")，页名保留前导零
pub fn jm_ids_from_url(url: &str) -> Option<(u64, String)> {
    let path = url.split(['?', '#']).next()?;
    let mut segs = path.rsplit('/');
    let file = segs.next()?;
    let aid = segs.next()?.parse::<u64>().ok()?;
    let page = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    (!page.is_empty()).then(|| (aid, page.to_string()))
}

/// 18comic 一页图被切成几条横带
pub fn jm_block_count(aid: u64, page: &str) -> u32 {
    let hex = format!("{:x}", Md5::digest(format!("{aid}{page}").as_bytes()));
    let mut n = hex.chars().last().unwrap_or('0') as u32;
    if (268_850..=421_925).contains(&aid) {
        n %= 10;
    } else if aid >= 421_926 {
        n %= 8;
    }
    if n <= 9 {
        2 + 2 * n
    } else {
        10
    }
}

/// 一条横带的搬运：从源图 `src_y` 起取 `height` 行，放到目标的 `dst_y`
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Band {
    pub src_y: u32,
    pub dst_y: u32,
    pub height: u32,
}

/// 横带倒序：从底往上取、从顶往下放，最下面那条高 h+f，其余高 h（h = H / c，f = H % c）
pub fn jm_bands(height: u32, c: u32) -> Vec<Band> {
    let h = height / c;
    let f = height % c;
    (0..c)
        .map(|g| Band {
            src_y: height - h * (g + 1) - f,
            dst_y: if g == 0 { 0 } else { h * g + f },
            height: if g == 0 { h + f } else { h },
        })
        .collect()
}

/// 还原 18comic 打乱的一页；不需要还原时返回 `None`
pub fn descramble_jm(img: &DynamicImage, aid: u64, page: &str) -> Option<DynamicImage> {
    if aid < JM_SCRAMBLE_FROM {
        return None;
    }
    let (w, h) = img.dimensions();
    let c = jm_block_count(aid, page);
    if c < 2 || h < c {
        return None;
    }
    let mut out = DynamicImage::new(w, h, img.color());
    for b in jm_bands(h, c) {
        let part = img.crop_imm(0, b.src_y, w, b.height);
        out.copy_from(&part, 0, b.dst_y).ok()?;
    }
    Some(out)
}

/// 解码 → 还原 → 重新编码。image 库不能编码 webp，所以写成 JPEG（带透明通道时写 PNG）；
/// 文件扩展名由 `pack::image_ext` 按文件头决定，不受原地址影响
pub fn jm_fix_bytes(bytes: &[u8], aid: u64, page: &str) -> Option<Vec<u8>> {
    if aid < JM_SCRAMBLE_FROM {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let fixed = descramble_jm(&img, aid, page)?;
    let mut buf = Cursor::new(Vec::new());
    if fixed.color().has_alpha() {
        fixed.write_to(&mut buf, ImageFormat::Png).ok()?;
    } else {
        let rgb = DynamicImage::ImageRgb8(fixed.to_rgb8());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 92).encode_image(&rgb).ok()?;
    }
    Some(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    const URL: &str = "https://cdn-msp2.18comic.ink/media/photos/1222566/00002.webp";

    #[test]
    fn ids_from_url() {
        assert_eq!(jm_ids_from_url(URL), Some((1_222_566, "00002".to_string())));
        assert_eq!(
            jm_ids_from_url("https://cdn.example/media/photos/350000/00010.jpg?v=1"),
            Some((350_000, "00010".to_string()))
        );
        assert_eq!(jm_ids_from_url("https://cdn.example/media/photos/abc/00010.jpg"), None);
    }

    #[test]
    fn block_counts_match_site() {
        assert_eq!(jm_block_count(1_222_566, "00002"), 10);
        assert_eq!(jm_block_count(1_222_566, "00003"), 6);
    }

    #[test]
    fn bands_for_sample_page() {
        let want = [
            (603, 678, 0, 75),
            (536, 603, 75, 142),
            (469, 536, 142, 209),
            (402, 469, 209, 276),
            (335, 402, 276, 343),
            (268, 335, 343, 410),
            (201, 268, 410, 477),
            (134, 201, 477, 544),
            (67, 134, 544, 611),
            (0, 67, 611, 678),
        ];
        let got: Vec<_> = jm_bands(678, 10)
            .into_iter()
            .map(|b| (b.src_y, b.src_y + b.height, b.dst_y, b.dst_y + b.height))
            .collect();
        assert_eq!(got, want);
        // 00003：c=6，h=113，f=0，正好倒序
        let b = jm_bands(678, 6);
        assert_eq!(b.len(), 6);
        assert!(b.iter().all(|b| b.height == 113));
        assert_eq!((b[0].src_y, b[0].dst_y), (565, 0));
        assert_eq!((b[5].src_y, b[5].dst_y), (0, 565));
    }

    /// 每行像素记着它在原图里的行号；按网站的方式打乱，再还原，应当回到 0..H 连续
    fn scramble(orig: &RgbImage, c: u32) -> RgbImage {
        let (w, h) = orig.dimensions();
        let mut s = RgbImage::new(w, h);
        for b in jm_bands(h, c) {
            // 还原是 src→dst，打乱就是 dst→src
            for y in 0..b.height {
                for x in 0..w {
                    s.put_pixel(x, b.src_y + y, *orig.get_pixel(x, b.dst_y + y));
                }
            }
        }
        s
    }

    fn row_image(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |_, y| Rgb([(y >> 8) as u8, (y & 0xFF) as u8, 0]))
    }

    #[test]
    fn descramble_restores_rows() {
        for page in ["00002", "00003"] {
            let c = jm_block_count(1_222_566, page);
            let orig = row_image(4, 678);
            let scrambled = DynamicImage::ImageRgb8(scramble(&orig, c));
            let fixed = descramble_jm(&scrambled, 1_222_566, page).unwrap().to_rgb8();
            assert_eq!(fixed, orig, "page {page}");
        }
    }

    #[test]
    fn old_albums_pass_through() {
        let img = DynamicImage::ImageRgb8(row_image(4, 100));
        assert!(descramble_jm(&img, 220_980, "00001").is_none());
        assert!(jm_fix_bytes(b"whatever", 220_980, "00001").is_none());
    }

    #[test]
    fn fix_bytes_reencodes_webp_as_jpeg() {
        let orig = row_image(16, 678);
        let c = jm_block_count(1_222_566, "00002");
        let scrambled = DynamicImage::ImageRgb8(scramble(&orig, c));
        let mut webp = Cursor::new(Vec::new());
        scrambled.write_to(&mut webp, ImageFormat::WebP).unwrap();
        let out = fix_page("jm18comic", URL, webp.get_ref()).unwrap();
        assert_eq!(crate::fetch::pack::image_ext(URL, &out), "jpg");
        let back = image::load_from_memory(&out).unwrap().to_rgb8();
        assert_eq!(back.dimensions(), (16, 678));
        // JPEG 有损：只看每行大致对得上（绿色通道是行号低 8 位，蓝色恒为 0）
        for y in [0u32, 74, 75, 300, 610, 611, 677] {
            let p = back.get_pixel(8, y);
            let want = (y & 0xFF) as i32;
            assert!((p[1] as i32 - want).abs() <= 12, "row {y}: {:?}", p);
        }
        assert!(fix_page("unknown", URL, webp.get_ref()).is_none());
    }
}
