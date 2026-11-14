//! [local PoC patch — not upstream] 中继侧媒体音频转码：LPCM(S16LE) → Opus。
//!
//! 背景：iPhone 的媒体音频以 LPCM 48 kHz/2ch 到达，是 **1657 kbps 的固定开销**。在窄链路或
//! 与视频抢带宽时它既装不下、又会被挤掉。把能力广告改成 AAC 已被实测否证（手机干脆不发音频），
//! 所以唯一的出路是中继自己转码：Opus 96 kbps 大约是原来的 1/17。
//!
//! 只在 `OXP_TRANSCODE_OPUS=1` 时启用；未设 = 上游 LPCM 直通行为。

use opus::{Application, Bitrate, Channels, Encoder};

/// 本地补丁：不引入 anyhow（该 crate 未依赖它），错误就用字符串。
pub type TranscoderResult<T> = std::result::Result<T, String>;

/// 20 ms @ 48 kHz。Opus 允许 10/20/40/60 ms，20 ms 是延迟与效率的常用折中。
pub const OPUS_FRAME_SAMPLES: usize = 960;
/// 96 kbps 立体声对音乐类内容足够；链路极窄时可下调到 64 kbps。
pub const OPUS_BITRATE: i32 = 96_000;
/// 单包上限（Opus 帧最大远小于此值，留足余量避免截断）。
const OPUS_MAX_PACKET: usize = 4_000;

pub struct OpusTranscoder {
    encoder: Encoder,
    channels: usize,
    sample_rate: u32,
    /// 尚未凑满一帧的交错 i16 样本。
    pending: Vec<i16>,
    /// `pending` 里第一个样本对应的时间戳（µs）。
    base_pts_us: Option<i64>,
}

impl OpusTranscoder {
    pub fn new(sample_rate: u32, channels: usize, bitrate: i32) -> TranscoderResult<Self> {
        let channels = channels.max(1);
        let ch = if channels == 1 {
            Channels::Mono
        } else {
            Channels::Stereo
        };
        let mut encoder = Encoder::new(sample_rate, ch, Application::Audio)
            .map_err(|e| format!("opus encoder init: {e}"))?;
        encoder
            .set_bitrate(Bitrate::Bits(bitrate))
            .map_err(|e| format!("opus bitrate: {e}"))?;
        Ok(Self {
            encoder,
            channels,
            sample_rate,
            pending: Vec::with_capacity(OPUS_FRAME_SAMPLES * channels * 2),
            base_pts_us: None,
        })
    }

    fn frame_us(&self) -> i64 {
        (OPUS_FRAME_SAMPLES as i64) * 1_000_000 / self.sample_rate as i64
    }

    /// 喂一段 S16LE（交错）PCM，返回凑满的 Opus 帧：`(opus_payload, pts_us)`。
    /// 不足一帧的尾巴留在内部，等下一个包补齐 —— 因此传入的 `pts_us` 只作为该段首样本的时间。
    pub fn push(&mut self, pcm_le: &[u8], pts_us: i64) -> Vec<(Vec<u8>, i64)> {
        if self.base_pts_us.is_none() {
            self.base_pts_us = Some(pts_us);
        }
        self.pending.extend(
            pcm_le
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]])),
        );

        let frame_len = OPUS_FRAME_SAMPLES * self.channels;
        let frame_us = self.frame_us();
        let mut out = Vec::new();
        let mut emitted = 0i64;
        while self.pending.len() >= frame_len {
            let frame: Vec<i16> = self.pending.drain(..frame_len).collect();
            match self.encoder.encode_vec(&frame, OPUS_MAX_PACKET) {
                Ok(bytes) if !bytes.is_empty() => {
                    let pts = self.base_pts_us.unwrap_or(pts_us) + emitted * frame_us;
                    out.push((bytes, pts));
                    emitted += 1;
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        if let Some(base) = self.base_pts_us {
            self.base_pts_us = Some(base + emitted * frame_us);
        }
        out
    }
}

/// 部署开关：`OXP_TRANSCODE_OPUS=1` 才把媒体 LPCM 转成 Opus。
pub fn opus_enabled() -> bool {
    matches!(std::env::var("OXP_TRANSCODE_OPUS").as_deref(), Ok("1") | Ok("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcodes_one_second_of_pcm_into_fifty_opus_frames() {
        let mut t = OpusTranscoder::new(48_000, 2, OPUS_BITRATE).unwrap();
        let mut packets = Vec::new();
        // 50 个 20 ms 的包，每包 960 帧 × 2 声道 × 2 字节
        for i in 0..50i64 {
            let pcm = vec![0u8; OPUS_FRAME_SAMPLES * 2 * 2];
            packets.extend(t.push(&pcm, i * 20_000));
        }
        assert_eq!(packets.len(), 50, "1 秒的 PCM 应恰好切成 50 个 20 ms 帧");
        assert!(packets.iter().all(|(b, _)| !b.is_empty() && b.len() < OPUS_MAX_PACKET));
        // 每包 2400 字节（20 ms 立体声 16 bit → 1920 字节）明显被压缩
        assert!(packets[0].0.len() < 1920, "Opus 帧应远小于原始 PCM 帧");
        assert_eq!(packets[1].1 - packets[0].1, 20_000, "pts 按 20 ms 递进");
    }

    #[test]
    fn keeps_partial_frame_between_pushes() {
        let mut t = OpusTranscoder::new(48_000, 2, OPUS_BITRATE).unwrap();
        // 半帧 = 480 样本 × 2 声道 × 2 字节
        let half = vec![0u8; OPUS_FRAME_SAMPLES * 2];
        assert!(t.push(&half, 0).is_empty(), "不足一帧不应产出");
        let second = t.push(&half, 10_000);
        assert_eq!(second.len(), 1, "补齐后应产出 1 帧");
    }
}
