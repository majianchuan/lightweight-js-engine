//! Date built-in. Local time is treated as UTC for deterministic behaviour.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use indexmap::IndexMap;

const MS_PER_DAY: f64 = 86_400_000.0;

impl Interp {
    pub fn install_date(&mut self) {
        let proto = self.realm.intrinsics.date_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "Date",
            7,
            |interp, _this, _args| {
                let now = now_ms();
                Ok(Value::String(JsString::from_str(&format_date_to_string(
                    now,
                ))))
            },
            |interp, args, new_target| {
                let value = if args.is_empty() {
                    now_ms()
                } else if args.len() == 1 {
                    let v = args[0].clone();
                    if let Some(s) = v.as_string() {
                        parse_date_string(&s.to_string_lossy())
                    } else {
                        let prim = interp.to_primitive(&v, "default")?;
                        if let Some(s) = prim.as_string() {
                            parse_date_string(&s.to_string_lossy())
                        } else {
                            let n = interp.to_number(&prim)?;
                            time_clip(n)
                        }
                    }
                } else {
                    let year = interp.to_number(&args[0])?;
                    let month = interp.to_number(&args[1])?;
                    let day = if args.len() > 2 {
                        interp.to_number(&args[2])?
                    } else {
                        1.0
                    };
                    let hours = if args.len() > 3 {
                        interp.to_number(&args[3])?
                    } else {
                        0.0
                    };
                    let minutes = if args.len() > 4 {
                        interp.to_number(&args[4])?
                    } else {
                        0.0
                    };
                    let seconds = if args.len() > 5 {
                        interp.to_number(&args[5])?
                    } else {
                        0.0
                    };
                    let ms = if args.len() > 6 {
                        interp.to_number(&args[6])?
                    } else {
                        0.0
                    };
                    make_date(year, month, day, hours, minutes, seconds, ms)
                };
                let p = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::Date(value),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("Date", Value::Object(ctor.clone()));
        self.realm.intrinsics.date_ctor = Some(ctor.clone());

        self.define_method(&ctor, "now", 0, |_interp, _this, _args| {
            Ok(Value::Number(now_ms()))
        });
        self.define_method(&ctor, "parse", 1, |interp, _this, args| {
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Number(parse_date_string(&s.to_string_lossy())))
        });
        self.define_method(&ctor, "UTC", 7, |interp, _this, args| {
            let mut num = |i: usize, default: f64| -> JsResult<f64> {
                match args.get(i) {
                    Some(v) if !v.is_undefined() => interp.to_number(v),
                    _ => Ok(default),
                }
            };
            let year = num(0, f64::NAN)?;
            let month = num(1, 0.0)?;
            let day = num(2, 1.0)?;
            let hours = num(3, 0.0)?;
            let minutes = num(4, 0.0)?;
            let seconds = num(5, 0.0)?;
            let ms = num(6, 0.0)?;
            Ok(Value::Number(make_date(
                year, month, day, hours, minutes, seconds, ms,
            )))
        });

        self.define_method(&proto, "getTime", 0, |interp, this, _args| {
            Ok(Value::Number(interp.date_value(this)?))
        });
        self.define_method(&proto, "valueOf", 0, |interp, this, _args| {
            Ok(Value::Number(interp.date_value(this)?))
        });
        self.define_method(&proto, "getTimezoneOffset", 0, |_interp, _this, _args| {
            Ok(Value::Number(0.0))
        });
        let getters: Vec<(&str, fn(f64) -> f64)> = vec![
            ("getFullYear", |t| year_from_time(t) as f64),
            ("getMonth", |t| month_from_time(t) as f64),
            ("getDate", |t| date_from_time(t) as f64),
            ("getDay", |t| day_from_time(t) as f64),
            ("getHours", |t| hour_from_time(t) as f64),
            ("getMinutes", |t| minute_from_time(t) as f64),
            ("getSeconds", |t| second_from_time(t) as f64),
            ("getMilliseconds", |t| ms_from_time(t) as f64),
            ("getYear", |t| year_from_time(t) as f64 - 1900.0),
        ];
        for (name, f) in getters {
            self.define_method(&proto, name, 0, move |interp, this, _args| {
                let t = interp.date_value(this)?;
                if t.is_nan() {
                    Ok(Value::Number(f64::NAN))
                } else {
                    Ok(Value::Number(f(t)))
                }
            });
            let utc_name = format!("getUTC{}", &name[3..]);
            self.define_method(&proto, &utc_name, 0, move |interp, this, _args| {
                let t = interp.date_value(this)?;
                if t.is_nan() {
                    Ok(Value::Number(f64::NAN))
                } else {
                    Ok(Value::Number(f(t)))
                }
            });
        }
        self.define_method(&proto, "setTime", 1, |interp, this, args| {
            let t = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            let t = time_clip(t);
            interp.set_date_value(this, t)?;
            Ok(Value::Number(t))
        });
        let setters: Vec<(&str, fn(f64, &[f64]) -> f64)> = vec![
            ("setMilliseconds", |t, a| set_time_fields(t, 0, a, 3)),
            ("setSeconds", |t, a| set_time_fields(t, 1, a, 3)),
            ("setMinutes", |t, a| set_time_fields(t, 2, a, 3)),
            ("setHours", |t, a| set_time_fields(t, 3, a, 4)),
            ("setDate", |t, a| set_time_fields(t, 4, a, 1)),
            ("setMonth", |t, a| set_time_fields(t, 5, a, 2)),
            ("setFullYear", |t, a| set_time_fields(t, 6, a, 3)),
        ];
        for (name, f) in setters {
            self.define_method(&proto, name, 1, move |interp, this, args| {
                let t = interp.date_value(this)?;
                let mut nums = Vec::new();
                for a in args {
                    nums.push(interp.to_number(a)?);
                }
                let v = f(t, &nums);
                let v = time_clip(v);
                interp.set_date_value(this, v)?;
                Ok(Value::Number(v))
            });
            let utc_name = format!("setUTC{}", &name[3..]);
            self.define_method(&proto, &utc_name, 1, move |interp, this, args| {
                let t = interp.date_value(this)?;
                let mut nums = Vec::new();
                for a in args {
                    nums.push(interp.to_number(a)?);
                }
                let v = f(t, &nums);
                let v = time_clip(v);
                interp.set_date_value(this, v)?;
                Ok(Value::Number(v))
            });
        }
        self.define_method(&proto, "setYear", 1, |interp, this, args| {
            let t = interp.date_value(this)?;
            let y = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            let y = if (0.0..=99.0).contains(&y) {
                y + 1900.0
            } else {
                y
            };
            let v = time_clip(set_time_fields(t, 6, &[y], 1));
            interp.set_date_value(this, v)?;
            Ok(Value::Number(v))
        });
        self.define_method(&proto, "toString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            Ok(Value::String(JsString::from_str(&format_date_to_string(t))))
        });
        self.define_method(&proto, "toDateString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            Ok(Value::String(JsString::from_str(&format_date_part(t))))
        });
        self.define_method(&proto, "toTimeString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            Ok(Value::String(JsString::from_str(&format_time_part(
                t, true,
            ))))
        });
        self.define_method(&proto, "toUTCString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            Ok(Value::String(JsString::from_str(&format_utc_string(t))))
        });
        self.define_method(&proto, "toGMTString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            Ok(Value::String(JsString::from_str(&format_utc_string(t))))
        });
        self.define_method(&proto, "toISOString", 0, |interp, this, _args| {
            let t = interp.date_value(this)?;
            if t.is_nan() {
                return interp.throw_range_error("Invalid time value");
            }
            Ok(Value::String(JsString::from_str(&format_iso(t))))
        });
        self.define_method(&proto, "toJSON", 1, |interp, this, _args| {
            let t = interp.date_value(this)?;
            if t.is_nan() {
                return Ok(Value::Null);
            }
            Ok(Value::String(JsString::from_str(&format_iso(t))))
        });
        for name in ["toLocaleString", "toLocaleDateString", "toLocaleTimeString"] {
            self.define_method(&proto, name, 0, |interp, this, _args| {
                let t = interp.date_value(this)?;
                let s = match t.is_nan() {
                    true => "Invalid Date".to_string(),
                    false => format_date_to_string(t),
                };
                Ok(Value::String(JsString::from_str(&s)))
            });
        }
        let prim = self.native_method("[Symbol.toPrimitive]", 1, |interp, this, args| {
            let hint = args
                .first()
                .and_then(|v| v.as_string())
                .map(|s| s.to_string_lossy())
                .unwrap_or_default();
            let t = interp.date_value(this)?;
            if hint == "number" {
                Ok(Value::Number(t))
            } else {
                Ok(Value::String(JsString::from_str(&format_date_to_string(t))))
            }
        });
        let desc = Descriptor {
            value: Some(Value::Object(prim)),
            writable: Some(false),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(
            &proto,
            PropertyKey::Symbol(self.realm.intrinsics.symbol_to_primitive.clone()),
            desc,
        );
    }

    pub fn date_value(&mut self, this: &Value) -> JsResult<f64> {
        if let Some(o) = this.as_object_cloned() {
            if let ObjectKind::Date(t) = &o.borrow().kind {
                return Ok(*t);
            }
        }
        self.throw_type_error("this is not a Date object")
    }

    pub fn set_date_value(&mut self, this: &Value, value: f64) -> JsResult<()> {
        if let Some(o) = this.as_object_cloned() {
            if let ObjectKind::Date(t) = &mut o.borrow_mut().kind {
                *t = value;
                return Ok(());
            }
        }
        self.throw_type_error("this is not a Date object")
    }
}

fn now_ms() -> f64 {
    let now = Utc::now();
    now.timestamp_millis() as f64
}

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > 8_640_000_000_000_000.0 {
        f64::NAN
    } else if t == 0.0 {
        0.0
    } else {
        t.trunc()
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn make_date(year: f64, month: f64, day: f64, h: f64, min: f64, sec: f64, ms: f64) -> f64 {
    if !year.is_finite() || !month.is_finite() || !day.is_finite() {
        return f64::NAN;
    }
    let mut y = year.trunc() as i64;
    if (0..=99).contains(&y) {
        y += 1900;
    }
    let mut m = month.trunc() as i64;
    let mut d = day.trunc() as i64;
    let mut hh = h.trunc() as i64;
    let mut mm = min.trunc() as i64;
    let mut ss = sec.trunc() as i64;
    let mmm = ms.trunc() as i64;
    if !year.is_finite() || m.abs() > 1_000_000 || d.abs() > 100_000_000 {
        return f64::NAN;
    }
    y += m.div_euclid(12);
    m = m.rem_euclid(12);
    if y.abs() > 275_760 || d.abs() > 100_000_000 {
        return f64::NAN;
    }
    let days = days_from_civil(y, m + 1, 1) + (d - 1);
    let total = days * 86_400_000 + hh * 3_600_000 + mm * 60_000 + ss * 1000 + mmm;
    let _ = &mut hh;
    let _ = &mut mm;
    let _ = &mut ss;
    time_clip(total as f64)
}

fn split_time(t: f64) -> Option<(i64, i64, i64, i64, i64, i64, i64, i64)> {
    if !t.is_finite() {
        return None;
    }
    let t = t.trunc() as i64;
    let days = t.div_euclid(86_400_000);
    let rem = t.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3_600_000;
    let mm = (rem / 60_000) % 60;
    let ss = (rem / 1000) % 60;
    let ms = rem % 1000;
    let weekday = (days + 4).rem_euclid(7);
    Some((y, m, d, hh, mm, ss, ms, weekday))
}

fn year_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.0 as f64).unwrap_or(f64::NAN)
}
fn month_from_time(t: f64) -> f64 {
    split_time(t).map(|x| (x.1 - 1) as f64).unwrap_or(f64::NAN)
}
fn date_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.2 as f64).unwrap_or(f64::NAN)
}
fn day_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.7 as f64).unwrap_or(f64::NAN)
}
fn hour_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.3 as f64).unwrap_or(f64::NAN)
}
fn minute_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.4 as f64).unwrap_or(f64::NAN)
}
fn second_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.5 as f64).unwrap_or(f64::NAN)
}
fn ms_from_time(t: f64) -> f64 {
    split_time(t).map(|x| x.6 as f64).unwrap_or(f64::NAN)
}

fn set_time_fields(t: f64, field: usize, args: &[f64], _max: usize) -> f64 {
    if args.is_empty() {
        return f64::NAN;
    }
    let (mut y, mut mo, mut d, mut h, mut mi, mut s, mut ms, _) = match split_time(t) {
        Some(v) => v,
        None => (1970, 1, 1, 0, 0, 0, 0, 4),
    };
    if field == 6 {
        y = args[0].trunc() as i64;
        if (0..=99).contains(&y) {
            y += 1900;
        }
        if args.len() > 1 {
            mo = args[1].trunc() as i64 + 1;
        }
        if args.len() > 2 {
            d = args[2].trunc() as i64;
        }
    } else if field == 5 {
        mo = args[0].trunc() as i64 + 1;
        if args.len() > 1 {
            d = args[1].trunc() as i64;
        }
    } else if field == 4 {
        d = args[0].trunc() as i64;
    } else if field == 3 {
        h = args[0].trunc() as i64;
        if args.len() > 1 {
            mi = args[1].trunc() as i64;
        }
        if args.len() > 2 {
            s = args[2].trunc() as i64;
        }
        if args.len() > 3 {
            ms = args[3].trunc() as i64;
        }
    } else if field == 2 {
        mi = args[0].trunc() as i64;
        if args.len() > 1 {
            s = args[1].trunc() as i64;
        }
        if args.len() > 2 {
            ms = args[2].trunc() as i64;
        }
    } else if field == 1 {
        s = args[0].trunc() as i64;
        if args.len() > 1 {
            ms = args[1].trunc() as i64;
        }
    } else {
        ms = args[0].trunc() as i64;
    }
    if y.abs() > 275_760 {
        return f64::NAN;
    }
    let days = days_from_civil(y, ((mo - 1).rem_euclid(12)) + 1, 1) + d - 1;
    let total: i128 = (days as i128) * 86_400_000
        + (h as i128) * 3_600_000
        + (mi as i128) * 60_000
        + (s as i128) * 1000
        + (ms as i128);
    time_clip(total as f64)
}

fn format_iso(t: f64) -> String {
    match split_time(t) {
        Some((y, m, d, h, mi, s, ms, _)) => {
            let year = if (0..=9999).contains(&y) {
                format!("{:04}", y)
            } else if y < 0 {
                format!("-{:06}", -y)
            } else {
                format!("+{:06}", y)
            };
            format!(
                "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                year, m, d, h, mi, s, ms
            )
        }
        None => "Invalid Date".to_string(),
    }
}

fn weekday_name(w: i64) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][w.rem_euclid(7) as usize]
}

fn weekday_long(w: i64) -> &'static str {
    [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ][w.rem_euclid(7) as usize]
}

fn month_name(m: i64) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1).rem_euclid(12) as usize]
}

fn format_date_part(t: f64) -> String {
    match split_time(t) {
        Some((y, m, d, _, _, _, _, w)) => {
            format!("{} {} {:02} {}", weekday_name(w), month_name(m), d, y)
        }
        None => "Invalid Date".to_string(),
    }
}

fn format_time_part(t: f64, zone: bool) -> String {
    match split_time(t) {
        Some((_, _, _, h, mi, s, _, _)) => {
            if zone {
                format!(
                    "{:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
                    h, mi, s
                )
            } else {
                format!("{:02}:{:02}:{:02}", h, mi, s)
            }
        }
        None => "Invalid Date".to_string(),
    }
}

fn format_date_to_string(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    format!("{} {}", format_date_part(t), format_time_part(t, true))
}

fn format_utc_string(t: f64) -> String {
    match split_time(t) {
        Some((y, m, d, h, mi, s, _, w)) => format!(
            "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
            weekday_name(w),
            d,
            month_name(m),
            y,
            h,
            mi,
            s
        ),
        None => "Invalid Date".to_string(),
    }
}

fn parse_date_string(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return f64::NAN;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return time_clip(dt.timestamp_millis() as f64);
    }
    let formats = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%m/%d/%Y",
        "%B %d, %Y",
        "%b %d, %Y",
    ];
    for fmt in formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return time_clip(dt.and_utc().timestamp_millis() as f64);
        }
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            let dt = d.and_hms_opt(0, 0, 0).unwrap();
            return time_clip(dt.and_utc().timestamp_millis() as f64);
        }
    }
    // RFC 2822-ish: "Thu, 01 Jan 1970 00:00:00 GMT"
    for fmt in [
        "%a, %d %b %Y %H:%M:%S GMT",
        "%a %b %d %Y %H:%M:%S GMT",
        "%a, %d %b %Y %H:%M:%S",
        "%a %b %d %Y %H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return time_clip(dt.and_utc().timestamp_millis() as f64);
        }
    }
    // Local-ish strings without timezone: parse as UTC.
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%:z") {
        return time_clip(dt.timestamp_millis() as f64);
    }
    let _ = (
        Utc.timestamp_opt(0, 0),
        Datelike::year(&Utc::now()),
        Timelike::hour(&Utc::now()),
    );
    f64::NAN
}
