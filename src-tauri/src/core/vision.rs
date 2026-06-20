//! 多模态（视觉 / 图片识别）能力推断 —— 纯 Rust，不依赖 Tauri 类型。
//!
//! 与上下文窗口一样：没有标准接口能查询「某模型是否支持视觉输入」，官方
//! OpenAI / Anthropic 的 `/v1/models` 都不返回该能力。因此只能按模型名查表推断。
//!
//! 该值决定 `agents/llm.rs` 是否把会议室图片附件内联进请求（见其 `cfg.supports_vision`
//! 分支）。查不到一律按**不支持**（false）兜底——宁可不发图，避免给纯文本模型塞图片
//! 触发 4xx。仅在创建 LLM 配置与修改 model 时各自动推断一次。

/// 已知多模态模型的名称标记（小写子串匹配）。命中任一即判定支持视觉输入。
/// 只收明确支持视觉的主流模型族；未命中按不支持兜底。
const VISION_MARKERS: &[&str] = &[
    // OpenAI
    "gpt-4o",
    "gpt-4.1",
    "gpt-4-turbo",
    "gpt-4-vision",
    "chatgpt-4o",
    // Anthropic（Claude 3 及以上均为多模态；claude-2 及更早为纯文本）
    "claude-3",
    "claude-sonnet-4",
    "claude-opus-4",
    "claude-haiku-4",
    // Google Gemini（全系多模态）
    "gemini",
    // 阿里 Qwen 视觉系
    "qwen-vl",
    "qwen2-vl",
    "qwen2.5-vl",
    "qwen-omni",
    "qvq",
    // DeepSeek 视觉系
    "deepseek-vl",
    // 智谱 GLM 视觉系
    "glm-4v",
    // 开源视觉模型
    "llava",
    "pixtral",
    // xAI Grok 视觉系
    "grok-2-vision",
    "grok-vision",
    // 通用兜底标记
    "vision",
];

/// 按模型名推断是否支持多模态（视觉）输入。查不到按不支持（false）兜底。
pub fn supports_vision(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    VISION_MARKERS.iter().any(|marker| m.contains(marker))
}

// ── 多模态「读回式」探测用的图片合成 ──────────────────────────────────────────
//
// 「测试连接」时仅凭含图请求返回 HTTP 200 来判定多模态会产生假阳性：很多 OpenAI 兼容
// 网关对带 image_url 的请求照常 200，只是把图片 part 静默丢弃（如 mimo）。要可靠区分
// 「端点接受图像字段」与「模型真的看见了」，唯一办法是发一张含**随机数字**的图、让模型
// 读回再校验。无 image/png 依赖，故在此手写一个最小灰度 PNG 编码器（DEFLATE 仅用
// 「存储/未压缩」块，免 zlib 依赖）+ 5×7 点阵数字字模。纯 Rust，不引用 Tauri 类型。

/// 5×7 点阵数字字模（'1'=墨，'0'=底）。下标即数字本身。
const DIGIT_GLYPHS: [[&str; 7]; 10] = [
    ["01110", "10001", "10011", "10101", "11001", "10001", "01110"], // 0
    ["00100", "01100", "00100", "00100", "00100", "00100", "01110"], // 1
    ["01110", "10001", "00001", "00010", "00100", "01000", "11111"], // 2
    ["11111", "00010", "00100", "00010", "00001", "10001", "01110"], // 3
    ["00010", "00110", "01010", "10010", "11111", "00010", "00010"], // 4
    ["11111", "10000", "11110", "00001", "00001", "10001", "01110"], // 5
    ["00110", "01000", "10000", "11110", "10001", "10001", "01110"], // 6
    ["11111", "00001", "00010", "00100", "01000", "01000", "01000"], // 7
    ["01110", "10001", "10001", "01110", "10001", "10001", "01110"], // 8
    ["01110", "10001", "10001", "01111", "00001", "00010", "01100"], // 9
];

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
const MARGIN: usize = 3; // 逻辑像素留白
const GAP: usize = 2; // 数字间距（逻辑像素）
const SCALE: usize = 14; // 逻辑像素 → 物理像素放大倍数

/// 把一串数字渲染成黑字白底的灰度 PNG（字节流）。供多模态读回探测合成测试图。
/// 非数字字符被忽略；空输入回退为单个「0」字模，保证总能产出一张合法 PNG。
pub fn render_code_png(code: &str) -> Vec<u8> {
    let mut digits: Vec<usize> = code
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as usize))
        .collect();
    if digits.is_empty() {
        digits.push(0);
    }

    let n = digits.len();
    let logical_w = MARGIN * 2 + n * GLYPH_W + n.saturating_sub(1) * GAP;
    let logical_h = MARGIN * 2 + GLYPH_H;

    // 逻辑墨点位图：true=黑字。
    let mut ink = vec![vec![false; logical_w]; logical_h];
    for (i, &d) in digits.iter().enumerate() {
        let x0 = MARGIN + i * (GLYPH_W + GAP);
        for (row, pattern) in DIGIT_GLYPHS[d].iter().enumerate() {
            for (col, ch) in pattern.chars().enumerate() {
                if ch == '1' {
                    ink[MARGIN + row][x0 + col] = true;
                }
            }
        }
    }

    // 物理像素的「滤波字节 + 扫描行」原始数据（filter type 0 = None）。
    let phys_w = logical_w * SCALE;
    let phys_h = logical_h * SCALE;
    let mut raw = Vec::with_capacity(phys_h * (phys_w + 1));
    for y in 0..phys_h {
        raw.push(0u8); // 行滤波器：None
        let ly = y / SCALE;
        for x in 0..phys_w {
            let lx = x / SCALE;
            raw.push(if ink[ly][lx] { 0 } else { 255 });
        }
    }

    encode_grayscale_png(phys_w as u32, phys_h as u32, &raw)
}

/// 最小灰度（8-bit，color type 0）PNG 编码器。`raw` 须已含每行前缀滤波字节。
fn encode_grayscale_png(width: u32, height: u32, raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // PNG signature

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // bit depth 8, grayscale, deflate, filter 0, no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    let idat = zlib_stored(raw);
    write_chunk(&mut out, b"IDAT", &idat);

    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// 把数据包成 zlib 流，DEFLATE 仅用「存储（未压缩）」块——免 zlib/flate 依赖。
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    out.extend_from_slice(&[0x78, 0x01]); // zlib header（(0x78<<8|0x01) % 31 == 0）
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(0xFFFF).collect()
    };
    let last = chunks.len() - 1;
    for (i, block) in chunks.iter().enumerate() {
        out.push(if i == last { 1 } else { 0 }); // BFINAL + BTYPE=00（存储）
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct Crc32 {
    value: u32,
}

impl Crc32 {
    fn new() -> Self {
        Crc32 { value: 0xFFFF_FFFF }
    }
    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let mut c = (self.value ^ byte as u32) & 0xFF;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            self.value = c ^ (self.value >> 8);
        }
    }
    fn finish(self) -> u32 {
        self.value ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_and_chunks_present() {
        let png = render_code_png("4821");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        // 含 IHDR / IDAT / IEND 关键字
        let find = |needle: &[u8]| png.windows(needle.len()).any(|w| w == needle);
        assert!(find(b"IHDR") && find(b"IDAT") && find(b"IEND"));
    }

    #[test]
    fn adler_and_crc_known_values() {
        assert_eq!(adler32(b"abc"), 0x024d0127);
        let mut c = Crc32::new();
        c.update(b"123456789");
        assert_eq!(c.finish(), 0xCBF43926);
    }
}
