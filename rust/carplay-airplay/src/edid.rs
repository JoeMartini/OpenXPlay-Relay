//! [local PoC patch — not upstream] 面板 EDID 生成。
//!
//! 真车机在 `/info` 的 display 字典里带一份 `edid`（面板能力描述，含首选时序与刷新率）。
//! 我们之前没带，等于对 iPhone 说"这块屏没有面板描述"。这里按几何生成一份合法的
//! 128 字节 EDID 1.3 基础块，首选时序用当前几何 + 60 Hz。
//!
//! 校验：长度 128、头部 `00 FF FF FF FF FF FF 00`、单字节校验和使总和 mod 256 == 0。

/// 由像素几何与刷新率生成 128 字节 EDID 1.3 基础块。
///
/// 只填必备字段：厂商/产品标识、基本显示参数、色度、首选详细时序（DTD）、
/// 范围限制、显示器名、序列号。不附 CEA-861 扩展块（`extensions = 0`）。
pub fn panel_edid(width: u32, height: u32, refresh: u32) -> Vec<u8> {
    let mut e = vec![0u8; 128];
    // 0-7 固定头部
    e[0..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    // 8-9 厂商 ID "OXP"（5 bit 字母编码）
    let id = encode_mfg("OXP");
    e[8] = (id >> 8) as u8;
    e[9] = id as u8;
    // 10-11 产品码、12-15 序列号、16 生产周、17 生产年(1990+)
    e[10] = 0x01;
    e[11] = 0x00;
    e[12..16].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    e[16] = 1;
    e[17] = 30; // 2020
    // 18-19 EDID 版本 1.3
    e[18] = 1;
    e[19] = 3;
    // 20 视频输入：数字
    e[20] = 0x80;
    // 21-22 屏幕物理尺寸（cm）
    let wcm = (width as f64 / 720.0 * 17.0).round().clamp(1.0, 255.0) as u8;
    let hcm = ((height as f64 / width.max(1) as f64) * wcm as f64).round().clamp(1.0, 255.0) as u8;
    e[21] = wcm;
    e[22] = hcm;
    // 23 gamma 2.2
    e[23] = 0x78;
    // 24 特性：RGB 4:4:4 + 首选时序即原生时序
    e[24] = 0x0A;
    // 25-34 色度（典型 sRGB 近似值）
    e[25..35].copy_from_slice(&[0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]);
    // 35-37 已建立时序：不使用
    // 38-53 标准时序：不使用（0x01 0x01 = 未使用）
    for i in 0..8 {
        e[38 + i * 2] = 0x01;
        e[39 + i * 2] = 0x01;
    }
    // 54-71 首选详细时序
    let dtd = detailed_timing(width, height, refresh);
    e[54..72].copy_from_slice(&dtd);
    // 72-89 范围限制描述符
    let mut range = [0u8; 18];
    range[0..5].copy_from_slice(&[0x00, 0x00, 0x00, 0xFD, 0x00]);
    range[5] = 48; // 最低垂直频率 Hz
    range[6] = 90; // 最高垂直频率 Hz
    range[7] = 30; // 最低水平频率 kHz
    range[8] = 80; // 最高水平频率 kHz
    range[9] = 20; // 最大像素时钟 10 MHz 单位
    range[10] = 0x04;
    e[72..90].copy_from_slice(&range);
    // 90-107 显示器名
    e[90..108].copy_from_slice(&ascii_descriptor("OpenXPlay Relay"));
    // 108-125 序列号
    e[108..126].copy_from_slice(&ascii_descriptor("OXP-0001"));
    // 126 扩展块数量
    e[126] = 0;
    // 127 校验和
    let sum: u32 = e[0..127].iter().map(|b| *b as u32).sum();
    e[127] = (256 - (sum % 256)) as u8;
    e
}

fn encode_mfg(name: &str) -> u16 {
    let mut v: u16 = 0;
    for c in name.chars().take(3) {
        let code = (c.to_ascii_uppercase() as u16) - ('A' as u16) + 1;
        v = (v << 5) | (code & 0x1F);
    }
    v
}

/// 18 字节详细时序描述符（CVT/CEA 风格，用于 720p60 一类时序）。
fn detailed_timing(width: u32, height: u32, refresh: u32) -> [u8; 18] {
    // 按 CEA-861 的 720p/1080p 比例取消隐与同步宽度，按几何缩放
    let (hfp, hsw, hbp) = (110u32, 40u32, 220u32);
    let (vfp, vsw, vbp) = (5u32, 5u32, 20u32);
    let htotal = width + hfp + hsw + hbp;
    let vtotal = height + vfp + vsw + vbp;
    // 像素时钟（10 kHz 单位），按实际刷新率算
    let pixel_clock_khz = htotal as u64 * vtotal as u64 * refresh as u64 / 1000;
    let pclk = (pixel_clock_khz / 10).min(0xFFFF) as u16;
    let mut d = [0u8; 18];
    d[0] = (pclk & 0xFF) as u8;
    d[1] = (pclk >> 8) as u8;
    d[2] = (width & 0xFF) as u8;
    d[3] = ((htotal - width) & 0xFF) as u8;
    d[4] = (((width >> 8) as u8 & 0x0F) << 4) | ((htotal - width) >> 8) as u8 & 0x0F;
    d[5] = (height & 0xFF) as u8;
    d[6] = ((vtotal - height) & 0xFF) as u8;
    d[7] = (((height >> 8) as u8 & 0x0F) << 4) | ((vtotal - height) >> 8) as u8 & 0x0F;
    d[8] = (hfp & 0xFF) as u8;
    d[9] = (hsw & 0xFF) as u8;
    d[10] = (((vfp & 0x0F) << 4) | (vsw & 0x0F)) as u8;
    d[11] = ((((hfp >> 8) & 0x03) << 6) | (((hsw >> 8) & 0x03) << 4) | (((vfp >> 4) & 0x03) << 2) | ((vsw >> 4) & 0x03)) as u8;
    // 图像尺寸（mm），与面板物理尺寸一致
    let hmm = 170u32;
    let vmm = (hmm as f64 * height as f64 / width.max(1) as f64).round() as u32;
    d[12] = (hmm & 0xFF) as u8;
    d[13] = (vmm & 0xFF) as u8;
    d[14] = ((((hmm >> 8) & 0x0F) << 4) | ((vmm >> 8) & 0x0F)) as u8;
    d[15] = 0;
    d[16] = 0;
    d[17] = 0x1E; // 数字分离同步，行场均为正极性
    d
}

fn ascii_descriptor(text: &str) -> [u8; 18] {
    let mut d = [0u8; 18];
    d[0..5].copy_from_slice(&[0x00, 0x00, 0x00, 0xFC, 0x00]);
    let bytes = text.as_bytes();
    for i in 0..13 {
        d[5 + i] = *bytes.get(i).unwrap_or(&0x20);
    }
    d[17] = 0x0A;
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edid_is_well_formed_for_720p60() {
        let e = panel_edid(1280, 720, 60);
        assert_eq!(e.len(), 128);
        assert_eq!(&e[0..8], &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        assert_eq!(e[18], 1);
        assert_eq!(e[19], 3);
        let sum: u32 = e.iter().map(|b| *b as u32).sum();
        assert_eq!(sum % 256, 0, "EDID checksum must make total divisible by 256");
        // 首选详细时序：像素时钟 = 1650*750*60 = 74.25 MHz → 7425 (10 kHz)
        let pclk = e[54] as u32 | ((e[55] as u32) << 8);
        assert_eq!(pclk, 7425);
        let hactive = e[56] as u32 | (((e[58] as u32) >> 4) << 8);
        let vactive = e[59] as u32 | (((e[61] as u32) >> 4) << 8);
        assert_eq!((hactive, vactive), (1280, 720));
    }

    #[test]
    fn edid_scales_to_other_geometries() {
        let e = panel_edid(960, 540, 60);
        let sum: u32 = e.iter().map(|b| *b as u32).sum();
        assert_eq!(sum % 256, 0);
        let hactive = e[56] as u32 | (((e[58] as u32) >> 4) << 8);
        assert_eq!(hactive, 960);
    }
}
