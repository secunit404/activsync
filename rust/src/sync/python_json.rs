use crate::garmin_client::ActivityRecord;

/// The record's metrics as `json.dumps` writes them: only those present, in
/// this order, with Python's separators.
pub fn garmin_data_json(act: &ActivityRecord) -> String {
    let fields = [
        ("distance", &act.distance),
        ("duration", &act.duration),
        ("moving_duration", &act.moving_duration),
        ("elapsed_duration", &act.elapsed_duration),
        ("elevation_gain", &act.elevation_gain),
        ("elevation_loss", &act.elevation_loss),
        ("calories", &act.calories),
        ("avg_hr", &act.avg_hr),
        ("max_hr", &act.max_hr),
        ("avg_power", &act.avg_power),
        ("max_power", &act.max_power),
        ("norm_power", &act.norm_power),
        ("aerobic_te", &act.aerobic_te),
        ("anaerobic_te", &act.anaerobic_te),
        ("training_load", &act.training_load),
        ("avg_cadence", &act.avg_cadence),
        ("max_cadence", &act.max_cadence),
        ("total_sets", &act.total_sets),
        ("total_reps", &act.total_reps),
        ("total_volume", &act.total_volume),
    ];
    let present: Vec<String> = fields
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_ref()
                .map(|v| format!("\"{key}\": {}", py_number(v)))
        })
        .collect();
    format!("{{{}}}", present.join(", "))
}

/// Python's `repr()` of a JSON number: integers as is, floats with the
/// shortest round-trip digits, positional for exponents -4..16 (always with a
/// fractional part), otherwise `1e-05` / `1e+16` style.
fn py_number(n: &serde_json::Number) -> String {
    let Some(value) = n.as_f64().filter(|_| n.is_f64()) else {
        return n.to_string();
    };
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".into()
        } else if value > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let (sign, mantissa) = mantissa
        .strip_prefix('-')
        .map_or(("", mantissa), |m| ("-", m));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let body = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        return format!("{sign}{body}");
    }
    let mantissa = if digits.len() > 1 {
        format!("{}.{}", &digits[..1], &digits[1..])
    } else {
        digits
    };
    let exp_sign = if exponent < 0 { "-" } else { "+" };
    format!("{sign}{mantissa}e{exp_sign}{:02}", exponent.abs())
}

#[cfg(test)]
mod tests {
    use super::py_number;

    #[test]
    fn numbers_read_as_python_writes_them() {
        let repr = |text: &str| py_number(&serde_json::from_str(text).unwrap());
        for (input, python) in [
            ("1e-05", "1e-05"),
            ("1e16", "1e+16"),
            ("0.0001", "0.0001"),
            ("123456789012345.6", "123456789012345.6"),
            ("2990", "2990"),
            ("151.0", "151.0"),
            ("3.4", "3.4"),
            ("-0.5", "-0.5"),
            ("1.5e300", "1.5e+300"),
            ("0.30000000000000004", "0.30000000000000004"),
            ("1e15", "1000000000000000.0"),
            ("-2.5e-07", "-2.5e-07"),
        ] {
            assert_eq!(repr(input), python, "{input}");
        }
    }
}
