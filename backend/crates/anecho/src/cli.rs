//! Command-line parsing helpers shared by the client subcommands.

use anecho_contract::v0 as pb;
use anyhow::{Context, bail};

/// `--signal` syntax: `sine:<hz>`, `dual:<f1>,<f2>[,<ratio_db>]`, `multi:<f1>,<f2>,...`,
/// `white`, `pink`, `periodic[:<frames>]`, `square:<hz>`.
pub fn parse_signal(s: &str) -> anyhow::Result<pb::generator::Signal> {
    use pb::generator::Signal as S;
    let (kind, args) = match s.split_once(':') {
        Some((k, a)) => (k.trim(), a.trim()),
        None => (s.trim(), ""),
    };
    let nums = |a: &str| -> anyhow::Result<Vec<f32>> {
        a.split(',')
            .filter(|p| !p.trim().is_empty())
            .map(|p| {
                p.trim()
                    .parse::<f32>()
                    .with_context(|| format!("bad number {p:?}"))
            })
            .collect()
    };
    Ok(match kind {
        "sine" => {
            let v = nums(args)?;
            let &[hz] = v.as_slice() else {
                bail!("sine:<hz>")
            };
            S::Sine(pb::generator::Sine {
                frequency_hz: hz,
                amplitude_dbfs: 0.0,
            })
        }
        "dual" => {
            let v = nums(args)?;
            let (f1, f2, ratio) = match v.as_slice() {
                [f1, f2] => (*f1, *f2, 12.04),
                [f1, f2, r] => (*f1, *f2, *r),
                _ => bail!("dual:<f1>,<f2>[,<ratio_db>]"),
            };
            S::DualTone(pb::generator::DualTone {
                f1_hz: f1,
                f2_hz: f2,
                ratio_db: ratio,
            })
        }
        "multi" => {
            let v = nums(args)?;
            if v.is_empty() {
                bail!("multi:<f1>,<f2>,...");
            }
            S::Multitone(pb::generator::Multitone {
                frequencies_hz: v,
                schroeder_phases: true,
            })
        }
        "white" | "pink" | "periodic" => {
            let period = if kind == "periodic" {
                if args.is_empty() {
                    16_384
                } else {
                    args.parse::<u32>().context("periodic:<frames>")?
                }
            } else {
                0
            };
            S::Noise(pb::generator::Noise {
                kind: if kind == "pink" {
                    pb::generator::NoiseKind::Pink
                } else {
                    pb::generator::NoiseKind::White
                } as i32,
                period_frames: period,
                seed: 0,
            })
        }
        "square" => {
            let v = nums(args)?;
            let &[hz] = v.as_slice() else {
                bail!("square:<hz>")
            };
            S::Square(pb::generator::Square { frequency_hz: hz })
        }
        other => {
            bail!("unknown signal {other:?} (sine, dual, multi, white, pink, periodic, square)")
        }
    })
}

/// `--level` syntax: `-20dBFS` (peak) or `-10dBV` (RMS); a bare number means dBFS.
pub fn parse_level(s: &str) -> anyhow::Result<pb::generator::Level> {
    let t = s.trim().to_ascii_lowercase();
    let (num, unit) = match t.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (&t[..i], t[i..].trim()),
        None => (t.as_str(), "dbfs"),
    };
    let value: f32 = num
        .trim()
        .parse()
        .with_context(|| format!("bad level {s:?}"))?;
    let unit = match unit {
        "dbfs" => pb::generator::level::Unit::PeakDbfs(value),
        "dbv" => pb::generator::level::Unit::DbvRms(value),
        other => bail!("unknown level unit {other:?} (dBFS or dBV)"),
    };
    Ok(pb::generator::Level { unit: Some(unit) })
}

/// Build the contract generator from the CLI flags (`--sine hz,dbfs` is the v0.1 alias).
pub fn generator(
    signal: Option<&str>,
    level: Option<&str>,
    sine_alias: Option<&str>,
) -> anyhow::Result<Option<pb::Generator>> {
    if let Some(s) = sine_alias {
        let (f, a) = s.split_once(',').context("--sine expects <hz>,<dbfs>")?;
        return Ok(Some(pb::Generator {
            signal: Some(pb::generator::Signal::Sine(pb::generator::Sine {
                frequency_hz: f.trim().parse()?,
                amplitude_dbfs: a.trim().parse()?,
            })),
            ..Default::default()
        }));
    }
    let Some(signal) = signal else {
        return Ok(None);
    };
    let level = parse_level(level.unwrap_or("-20dBFS"))?;
    Ok(Some(pb::Generator {
        signal: Some(parse_signal(signal)?),
        level: Some(level),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_levels_and_signals() {
        assert!(matches!(
            parse_level("-20dBFS").unwrap().unit,
            Some(pb::generator::level::Unit::PeakDbfs(v)) if v == -20.0
        ));
        assert!(matches!(
            parse_level("-10 dBV").unwrap().unit,
            Some(pb::generator::level::Unit::DbvRms(v)) if v == -10.0
        ));
        assert!(matches!(
            parse_signal("dual:60,7000").unwrap(),
            pb::generator::Signal::DualTone(d) if d.f1_hz == 60.0 && d.f2_hz == 7000.0
        ));
        assert!(parse_signal("triangle:3").is_err());
    }
}
