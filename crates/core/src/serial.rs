//! Excel **日期序列数** ↔ 公历(1900 日期系统)。
//!
//! Excel 把日期存成「自纪元起的天数」,小数部分是当天时间比例。它的纪元定义
//! 有个著名的历史包袱:Lotus 1-2-3 误认为 1900 年是闰年,Excel 为兼容而保留了
//! **虚构的 1900-02-29**(序列数 60)。因此序列数 ≤ 59 与 ≥ 61 的换算基准差一天,
//! 必须显式复刻,否则 1900 年初的日期会整体偏移。
//!
//! 换算用 Howard Hinnant 的 `days_from_civil` / `civil_from_days`(不引入 chrono)。
//!
//! 这里是**全仓唯一**的日历实现。此前 `numfmt`、`xlsx`、`formula::functions::datetime`
//! 各有一份:前两者靠注释声明「与 datetime 一致」,而 `xlsx` 实际用的是朴素的
//! `serial - 25569`,**不复刻闰年 bug** —— 同一个工作簿里,走 numfmt 格式码的日期和
//! 走 `Data::DateTime` 的日期在 1900-01/02 会显示成不同的天。

/// 序列数的安全范围。
///
/// 远超任何真实日期(Excel 自身上限是 2958465 = 9999-12-31),但足够小,
/// 使 `z + 719468` 之类的中间量不会在 i64 上溢出 —— 畸形文件里出现
/// `1e300` 这种值时,`as i64` 会饱和到 `i64::MAX`,不夹紧就会算出乱码日期或 panic。
const SERIAL_LIMIT: i64 = 100_000_000;

/// 公历 (y, m, d) → 距 1970-01-01 的天数(可为负)。
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 距 1970-01-01 的天数 → 公历 (y, m, d)。
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z.clamp(-SERIAL_LIMIT, SERIAL_LIMIT) + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1899-12-31 距 1970-01-01 的天数,作为序列数换算基准。
pub fn epoch_offset() -> i64 {
    days_from_civil(1899, 12, 31)
}

/// Excel 序列数(整数天)→ 公历。
pub fn serial_to_ymd(serial: i64) -> (i64, i64, i64) {
    let serial = serial.clamp(-SERIAL_LIMIT, SERIAL_LIMIT);
    if serial == 60 {
        return (1900, 2, 29); // 虚构的闰日
    }
    let naive = if serial >= 61 { serial - 1 } else { serial };
    civil_from_days(naive + epoch_offset())
}

/// 公历 → Excel 序列数(含 1900 闰年 bug 复刻)。
pub fn ymd_to_serial(y: i64, m: i64, d: i64) -> i64 {
    let naive = days_from_civil(y, m, d) - epoch_offset();
    // naive >= 60 对应 1900-03-01 及以后,补上虚构的 1900-02-29
    if naive >= 60 {
        naive + 1
    } else {
        naive
    }
}

/// Excel 序列数 → `YYYY-MM-DD`(有时分秒则追加 ` HH:MM:SS`)。
pub fn serial_to_string(serial: f64) -> String {
    let whole = serial.floor();
    // 先夹紧再转 i64:f64 → i64 的 `as` 对超范围值是饱和的,直接用会得到 i64::MAX
    let day = (whole.clamp(-SERIAL_LIMIT as f64, SERIAL_LIMIT as f64)) as i64;

    // 当天时间(四舍五入到秒);进位到次日时日期要跟着 +1
    let secs = ((serial - whole) * 86_400.0).round() as i64;
    let (secs, day) = if secs >= 86_400 {
        (secs - 86_400, day + 1)
    } else {
        (secs, day)
    };
    let (y, m, d) = serial_to_ymd(day);
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);

    if hh == 0 && mm == 0 && ss == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_common_dates() {
        for (serial, y, m, d) in [
            (1, 1900, 1, 1),
            (59, 1900, 2, 28),
            (61, 1900, 3, 1),
            (25569, 1970, 1, 1),
            (44197, 2021, 1, 1),
        ] {
            assert_eq!(serial_to_ymd(serial), (y, m, d), "serial {serial}");
            assert_eq!(ymd_to_serial(y, m, d), serial, "{y}-{m}-{d}");
        }
    }

    /// 序列数 60 是 Lotus 遗留的虚构闰日,Excel 至今照显示不误。
    #[test]
    fn reproduces_the_1900_leap_year_bug() {
        assert_eq!(serial_to_ymd(60), (1900, 2, 29));
        assert_eq!(serial_to_string(60.0), "1900-02-29");
        // 它两侧的日期必须连续
        assert_eq!(serial_to_string(59.0), "1900-02-28");
        assert_eq!(serial_to_string(61.0), "1900-03-01");
    }

    /// 早先 xlsx 侧用朴素的 `serial - 25569`,序列数 1..59 会整体早一天。
    #[test]
    fn early_1900_dates_match_excel_not_the_naive_epoch() {
        assert_eq!(serial_to_string(1.0), "1900-01-01");
        assert_eq!(serial_to_string(2.0), "1900-01-02");
    }

    #[test]
    fn formats_time_of_day_and_carries_to_next_day() {
        assert_eq!(serial_to_string(44197.5), "2021-01-01 12:00:00");
        // 23:59:59.9 四舍五入到 24:00:00 → 进位次日
        assert_eq!(
            serial_to_string(44197.0 + 86_399.9 / 86_400.0),
            "2021-01-02"
        );
    }

    /// 畸形文件里的极值不得 panic,也不得算出乱码日期。
    #[test]
    fn extreme_values_are_clamped_not_overflowing() {
        for v in [1e300_f64, -1e300, f64::MAX, f64::MIN] {
            let s = serial_to_string(v);
            assert!(s.len() >= 10, "{v} → {s}");
        }
    }
}
