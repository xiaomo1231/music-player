//! 各平台歌词的加解密：QQ 音乐 QRC（魔改三重 DES + zlib）、酷狗 KRC（固定密钥异或 + zlib）、
//! 网易云 eapi 请求参数（AES-128-ECB + MD5 签名）。
//! QRC 的 DES 实现移植自 LDDC（GPL-3.0，https://github.com/chenmozhijin/LDDC）core/decryptor/tripledes.py，
//! 它又参考了 QQMusicDecoder 的 C# 实现。S 盒里有两处和标准 DES 不同（sbox2、sbox4），是 QQ 音乐的实现如此，不能改。
use anyhow::{anyhow, Result};
use std::io::Read;

fn inflate(data: &[u8]) -> Result<String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

// ---------------------------------------------------------------- 酷狗 KRC

const KRC_KEY: &[u8] = b"@Gaw^2tGQ61-\xce\xd2ni";

/// base64 解码后的 KRC：去掉 4 字节文件头 “krc1”，逐字节异或密钥，再 zlib 解压
pub fn krc_decrypt(data: &[u8]) -> Result<String> {
    if data.len() <= 4 {
        return Err(anyhow!("KRC 数据太短"));
    }
    let x: Vec<u8> = data[4..].iter().enumerate().map(|(i, b)| b ^ KRC_KEY[i % KRC_KEY.len()]).collect();
    inflate(&x)
}

// ---------------------------------------------------------------- QQ 音乐 QRC

const QRC_KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";

#[rustfmt::skip]
const SBOX: [[u8; 64]; 8] = [
    [14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7,
     0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8,
     4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0,
     15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13],
    [15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10,
     3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1, 10, 6, 9, 11, 5,
     0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15,
     13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9],
    [10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8,
     13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1,
     13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7,
     1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12],
    [7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15,
     13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9,
     10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4,
     3, 15, 0, 6, 10, 10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14],
    [2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9,
     14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6,
     4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14,
     11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3],
    [12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11,
     10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8,
     9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6,
     4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13],
    [4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1,
     13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6,
     1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2,
     6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12],
    [13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7,
     1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2,
     7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8,
     2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11],
];

/// 从 8 字节块里取第 b 位（这套实现的字节序是每 4 字节倒过来），放到第 c 位
fn bitnum(a: &[u8], b: usize, c: u32) -> u32 {
    (((a[(b / 32) * 4 + 3 - (b % 32) / 8] >> (7 - b % 8)) & 1) as u32) << c
}

fn bitnum_intr(a: u32, b: u32, c: u32) -> u32 {
    ((a >> (31 - b)) & 1) << c
}

fn bitnum_intl(a: u32, b: u32, c: u32) -> u32 {
    (a.wrapping_shl(b) & 0x8000_0000) >> c
}

fn sbox_bit(a: u8) -> usize {
    ((a & 32) | ((a & 31) >> 1) | ((a & 1) << 4)) as usize
}

fn initial_permutation(d: &[u8]) -> (u32, u32) {
    let half = |ks: [usize; 4]| {
        let mut v = 0u32;
        let mut pos = 31u32;
        for k in ks {
            for j in (0..8).rev() {
                v |= bitnum(d, 8 * j + k, pos);
                pos = pos.wrapping_sub(1);
            }
        }
        v
    };
    (half([1, 3, 5, 7]), half([0, 2, 4, 6]))
}

fn inverse_permutation(s0: u32, s1: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    for m in 0..8u32 {
        let mut v = 0u32;
        for t in 0..4u32 {
            v |= bitnum_intr(s1, m + 8 * t, 7 - 2 * t) | bitnum_intr(s0, m + 8 * t, 6 - 2 * t);
        }
        out[((m + 4) % 8) as usize] = v as u8;
    }
    out
}

#[rustfmt::skip]
const P: [(u32, u32); 32] = [
    (15, 0), (6, 1), (19, 2), (20, 3), (28, 4), (11, 5), (27, 6), (16, 7),
    (0, 8), (14, 9), (22, 10), (25, 11), (4, 12), (17, 13), (30, 14), (9, 15),
    (1, 16), (7, 17), (23, 18), (13, 19), (31, 20), (26, 21), (2, 22), (8, 23),
    (18, 24), (12, 25), (29, 26), (5, 27), (21, 28), (10, 29), (3, 30), (24, 31),
];

fn f(state: u32, key: &[u8; 6]) -> u32 {
    let s = state;
    let t1 = bitnum_intl(s, 31, 0)
        | ((s & 0xf000_0000) >> 1)
        | bitnum_intl(s, 4, 5)
        | bitnum_intl(s, 3, 6)
        | ((s & 0x0f00_0000) >> 3)
        | bitnum_intl(s, 8, 11)
        | bitnum_intl(s, 7, 12)
        | ((s & 0x00f0_0000) >> 5)
        | bitnum_intl(s, 12, 17)
        | bitnum_intl(s, 11, 18)
        | ((s & 0x000f_0000) >> 7)
        | bitnum_intl(s, 16, 23);
    let t2 = bitnum_intl(s, 15, 0)
        | ((s & 0x0000_f000) << 15)
        | bitnum_intl(s, 20, 5)
        | bitnum_intl(s, 19, 6)
        | ((s & 0x0000_0f00) << 13)
        | bitnum_intl(s, 24, 11)
        | bitnum_intl(s, 23, 12)
        | ((s & 0x0000_00f0) << 11)
        | bitnum_intl(s, 28, 17)
        | bitnum_intl(s, 27, 18)
        | ((s & 0x0000_000f) << 9)
        | bitnum_intl(s, 0, 23);
    let l = [
        ((t1 >> 24) as u8) ^ key[0],
        ((t1 >> 16) as u8) ^ key[1],
        ((t1 >> 8) as u8) ^ key[2],
        ((t2 >> 24) as u8) ^ key[3],
        ((t2 >> 16) as u8) ^ key[4],
        ((t2 >> 8) as u8) ^ key[5],
    ];
    let st = ((SBOX[0][sbox_bit(l[0] >> 2)] as u32) << 28)
        | ((SBOX[1][sbox_bit(((l[0] & 0x03) << 4) | (l[1] >> 4))] as u32) << 24)
        | ((SBOX[2][sbox_bit(((l[1] & 0x0f) << 2) | (l[2] >> 6))] as u32) << 20)
        | ((SBOX[3][sbox_bit(l[2] & 0x3f)] as u32) << 16)
        | ((SBOX[4][sbox_bit(l[3] >> 2)] as u32) << 12)
        | ((SBOX[5][sbox_bit(((l[3] & 0x03) << 4) | (l[4] >> 4))] as u32) << 8)
        | ((SBOX[6][sbox_bit(((l[4] & 0x0f) << 2) | (l[5] >> 6))] as u32) << 4)
        | (SBOX[7][sbox_bit(l[5] & 0x3f)] as u32);
    P.iter().fold(0, |v, &(b, c)| v | bitnum_intl(st, b, c))
}

type Schedule = [[u8; 6]; 16];

fn crypt(input: &[u8], key: &Schedule) -> [u8; 8] {
    let (mut s0, mut s1) = initial_permutation(input);
    for k in key.iter().take(15) {
        let prev = s1;
        s1 = f(s1, k) ^ s0;
        s0 = prev;
    }
    s0 ^= f(s1, &key[15]);
    inverse_permutation(s0, s1)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Encrypt,
    Decrypt,
}

fn key_schedule(key: &[u8], mode: Mode) -> Schedule {
    const SHIFT: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
    #[rustfmt::skip]
    const PERM_C: [usize; 28] = [56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35];
    #[rustfmt::skip]
    const PERM_D: [usize; 28] = [62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12, 4, 27, 19, 11, 3];
    #[rustfmt::skip]
    const COMP: [u32; 48] = [13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40, 51, 30, 36,
                             46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31];
    let mut sch = [[0u8; 6]; 16];
    let mut c: u32 = (0..28).map(|i| bitnum(key, PERM_C[i], 31 - i as u32)).sum();
    let mut d: u32 = (0..28).map(|i| bitnum(key, PERM_D[i], 31 - i as u32)).sum();
    for (i, &s) in SHIFT.iter().enumerate() {
        c = (c.wrapping_shl(s) | (c >> (28 - s))) & 0xffff_fff0;
        d = (d.wrapping_shl(s) | (d >> (28 - s))) & 0xffff_fff0;
        let g = if mode == Mode::Decrypt { 15 - i } else { i };
        sch[g] = [0; 6];
        for j in 0..24 {
            sch[g][j / 8] |= bitnum_intr(c, COMP[j], 7 - (j as u32 % 8)) as u8;
        }
        for j in 24..48 {
            sch[g][j / 8] |= bitnum_intr(d, COMP[j] - 27, 7 - (j as u32 % 8)) as u8;
        }
    }
    sch
}

fn triple_schedule(mode: Mode) -> [Schedule; 3] {
    let k = QRC_KEY;
    match mode {
        Mode::Decrypt => [
            key_schedule(&k[16..], Mode::Decrypt),
            key_schedule(&k[8..], Mode::Encrypt),
            key_schedule(&k[0..], Mode::Decrypt),
        ],
        Mode::Encrypt => [
            key_schedule(&k[0..], Mode::Encrypt),
            key_schedule(&k[8..], Mode::Decrypt),
            key_schedule(&k[16..], Mode::Encrypt),
        ],
    }
}

fn triple(data: &[u8], mode: Mode) -> Vec<u8> {
    let sch = triple_schedule(mode);
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks(8) {
        let mut b = [0u8; 8];
        b[..block.len()].copy_from_slice(block);
        for s in &sch {
            b = crypt(&b, s);
        }
        out.extend_from_slice(&b);
    }
    out
}

/// QQ 音乐接口返回的十六进制密文 → QRC / LRC 明文
pub fn qrc_decrypt(hex_text: &str) -> Result<String> {
    let data = hex::decode(hex_text.trim())?;
    if data.is_empty() {
        return Err(anyhow!("QRC 为空"));
    }
    inflate(&triple(&data, Mode::Decrypt))
}

#[cfg(test)]
pub fn qrc_encrypt_for_test(plain: &str) -> String {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(plain.as_bytes()).unwrap();
    let mut data = z.finish().unwrap();
    while data.len() % 8 != 0 {
        data.push(0);
    }
    hex::encode_upper(triple(&data, Mode::Encrypt))
}

// ---------------------------------------------------------------- 网易云 eapi

const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";

/// eapi 请求体：`params=` + AES-128-ECB(路径-36cd479b6b5-参数JSON-36cd479b6b5-签名) 的大写十六进制。
/// `path` 是把 /eapi/ 换成 /api/ 之后的路径
pub fn eapi_params(path: &str, json: &str) -> String {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    use md5::{Digest, Md5};
    let sign = hex::encode(Md5::digest(format!("nobody{path}use{json}md5forencrypt").as_bytes()));
    let mut data = format!("{path}-36cd479b6b5-{json}-36cd479b6b5-{sign}").into_bytes();
    let pad = 16 - data.len() % 16;
    data.resize(data.len() + pad, pad as u8);
    let cipher = aes::Aes128::new(GenericArray::from_slice(EAPI_KEY));
    for block in data.chunks_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
    format!("params={}", hex::encode_upper(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn krc_roundtrip() {
        use std::io::Write;
        let plain = "[0,1000]<0,500,0>你<500,500,0>好";
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(plain.as_bytes()).unwrap();
        let mut data = b"krc1".to_vec();
        data.extend(z.finish().unwrap().iter().enumerate().map(|(i, b)| b ^ KRC_KEY[i % KRC_KEY.len()]));
        assert_eq!(krc_decrypt(&data).unwrap(), plain);
    }

    /// 测试向量由 LDDC 的 Python 实现生成（tripledes_crypt + ENCRYPT 模式），保证移植结果和原实现逐位一致
    #[test]
    fn qrc_matches_reference() {
        let block = triple(b"ABCDEFGH", Mode::Encrypt);
        assert_eq!(hex::encode(&block), REF_BLOCK);
        assert_eq!(triple(&block, Mode::Decrypt), b"ABCDEFGH");
        let plain = "<?xml version=\"1.0\"?><Lyric_1 LyricType=\"1\" LyricContent=\"[0,1000]你(0,500)好(500,500)\"/>";
        assert_eq!(qrc_decrypt(&qrc_encrypt_for_test(plain)).unwrap(), plain);
    }
    const REF_BLOCK: &str = "fe1dd2e23bbba6e8";

    /// 测试向量由 LDDC 的 Python eapi_params_encrypt 生成
    #[test]
    fn eapi_matches_reference() {
        assert_eq!(eapi_params("/api/song/lyric/v1", r#"{"id":186016}"#), REF_EAPI);
    }
    const REF_EAPI: &str = "params=04AE33D34A93FE3EC22DA8FA305D290AB337D0FE5F36D211DE0D338CC6AA89D0B4B8A4386C9B6B36FA228CC253EA106F036B24621B7C3C73E90C94B814C08B13F29FB56A747CEC1B1A7C82C228C46137ECA7351E94C5674CB6C6B92CF1BBFF4F";
}
