// [local PoC patch — not upstream] 音频缺口判定：别把抖动当缺口、别把暂停当缺口。
use openxplay_relay::linux_audio::gap_samples;

#[test]
fn continuous_stream_has_no_gap() {
    assert_eq!(gap_samples(1_000, 640, 360, 48_000), 0);
}

#[test]
fn real_hole_is_measured_in_samples() {
    // 前一包 360 采样，时间戳推进 840 ⇒ 缺 480 采样 = 10 ms @48k
    assert_eq!(gap_samples(840, 0, 360, 48_000), 480);
}

#[test]
fn jitter_and_pauses_are_not_gaps() {
    assert_eq!(gap_samples(456, 0, 360, 48_000), 0);              // 2 ms 抖动
    assert_eq!(gap_samples(3 * 48_000, 0, 360, 48_000), 0);       // 3 s 暂停
}

#[test]
fn timestamp_wraparound_is_not_a_gap() {
    assert_eq!(gap_samples(259, u32::MAX - 100, 360, 48_000), 0);
}
