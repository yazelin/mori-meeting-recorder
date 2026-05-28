//! Audio level computation — peak + RMS in dB, given to VU meter.
//!
//! Pure functions, easily testable.

const DB_FLOOR: f32 = -120.0;
const SILENCE_LINEAR: f32 = 1e-6;

/// Linear amplitude → dB. Input <= SILENCE_LINEAR clamps to DB_FLOOR.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= SILENCE_LINEAR {
        DB_FLOOR
    } else {
        20.0 * linear.log10()
    }
}

/// Compute (peak_db, rms_db) from a batch of f32 samples (normalized to ±1.0).
/// Empty slice → (DB_FLOOR, DB_FLOOR).
pub fn compute_levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (DB_FLOOR, DB_FLOOR);
    }
    let mut peak_lin: f32 = 0.0;
    let mut sumsq: f64 = 0.0;
    for &s in samples {
        let abs = s.abs();
        if abs > peak_lin {
            peak_lin = abs;
        }
        sumsq += (s as f64) * (s as f64);
    }
    let rms_lin = (sumsq / samples.len() as f64).sqrt() as f32;
    (linear_to_db(peak_lin), linear_to_db(rms_lin))
}

/// Fast attack, slow release smoothing — VU meter standard ballistics。
///
/// 用途:每 50ms chunk 算出的 raw RMS / peak 在語音 inter-syllabic 停頓會 jitter
/// 到 -70 ~ -80 dB,套用 smoothing 後顯示值平緩下降,不再閃爍。
///
/// - `raw > prev` → 立刻 snap 到 raw(攻擊速度 = 1 個 frame)
/// - `raw ≤ prev` → prev 線性下降 `release_db_per_sec * dt_ms / 1000` dB,不低於 raw
///
/// 預設 release = 30 dB/秒:30dB drop ~1 秒,300ms 內 -9dB,500ms 內 -15dB。語音
/// inter-syllabic 停頓 < 300ms 完全被 smooth 過去,長停頓才看得到 bar 緩降。
pub fn smooth_db(prev: f32, raw: f32, release_db_per_sec: f32, dt_ms: f32) -> f32 {
    if raw > prev {
        raw
    } else {
        let release = release_db_per_sec * dt_ms / 1000.0;
        (prev - release).max(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_to_db_full_scale_is_zero() {
        assert!((linear_to_db(1.0) - 0.0).abs() < 0.01);
    }

    #[test]
    fn linear_to_db_silence_floor() {
        // 1e-6 should clamp to ~-120 dB (avoid log(0))
        assert!((linear_to_db(1e-6) - (-120.0)).abs() < 0.5);
    }

    #[test]
    fn linear_to_db_half_scale() {
        // 0.5 → 20 * log10(0.5) ≈ -6.02 dB
        assert!((linear_to_db(0.5) - (-6.02)).abs() < 0.05);
    }

    #[test]
    fn compute_levels_empty_returns_silence() {
        let (peak, rms) = compute_levels(&[]);
        assert!(peak < -100.0);
        assert!(rms < -100.0);
    }

    #[test]
    fn compute_levels_full_scale_sine_approx() {
        // 1024 samples of sin near amplitude 1.0
        let samples: Vec<f32> = (0..1024)
            .map(|i| (i as f32 * 0.1).sin())
            .collect();
        let (peak, rms) = compute_levels(&samples);
        // sine peak ≈ 1.0 → 0 dB (allow 1 dB tolerance)
        assert!(peak.abs() < 1.0, "peak={peak}, expected ~0 dB");
        // sine RMS = 1/sqrt(2) ≈ 0.707 → -3.01 dB
        assert!((rms - (-3.01)).abs() < 0.5, "rms={rms}, expected ~-3 dB");
    }

    #[test]
    fn compute_levels_dc_offset_only_rms_zero_peak_matches() {
        // All 0.5 → peak = 0.5 → ~-6 dB, RMS also ~-6 dB
        let samples = vec![0.5_f32; 1000];
        let (peak, rms) = compute_levels(&samples);
        assert!((peak - (-6.02)).abs() < 0.1);
        assert!((rms - (-6.02)).abs() < 0.1);
    }

    #[test]
    fn smooth_db_attack_is_instant() {
        // -60 → -20:raw > prev,直接 snap 到 raw
        assert_eq!(smooth_db(-60.0, -20.0, 30.0, 50.0), -20.0);
    }

    #[test]
    fn smooth_db_release_decays_linearly() {
        // -20 → -40,release 30 dB/s,dt=50ms = 1.5 dB decay
        // prev=-20 → -20 - 1.5 = -21.5,還沒到 -40
        let result = smooth_db(-20.0, -40.0, 30.0, 50.0);
        assert!((result - (-21.5)).abs() < 0.01);
    }

    #[test]
    fn smooth_db_release_clamps_to_raw() {
        // 大 dt → decay 算出來 >= prev - raw,結果應該 clamp 到 raw
        let result = smooth_db(-20.0, -25.0, 30.0, 1000.0);
        assert_eq!(result, -25.0);
    }

    #[test]
    fn smooth_db_300ms_silence_keeps_bar_visible() {
        // 模擬語音場景:start at -30(講話 peak),raw 連續 6 個 50ms chunk = -70(停頓)
        // smoothed 應該在 300ms 後還在 -39(明顯比 -70 高,bar 還亮著)
        let mut current = -30.0;
        for _ in 0..6 {
            current = smooth_db(current, -70.0, 30.0, 50.0);
        }
        // 6 * 1.5 = 9 dB decay,-30 → -39
        assert!((current - (-39.0)).abs() < 0.5, "current={current}");
    }
}
