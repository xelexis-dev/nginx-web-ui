use super::{ControlError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const CACHE_MAX_SIZES: &[&str] = &["100m", "500m", "1g", "5g"];
pub const CACHE_INACTIVES: &[&str] = &["10m", "60m", "24h"];
pub const CACHE_VALIDS: &[&str] = &["1m", "10m", "1h", "24h"];
pub const GZIP_TYPES_PRESETS: &[&str] = &["text", "text-and-fonts"];
pub const GZIP_MIN_LENGTH_MAX: u32 = 1048576;
pub const GZIP_COMP_LEVEL_MIN: u32 = 1;
pub const GZIP_COMP_LEVEL_MAX: u32 = 9;
pub const GZIP_TEXT_TYPES: &str = "text/css text/plain text/xml application/javascript application/json application/xml image/svg+xml";
pub const GZIP_TEXT_AND_FONTS_TYPES: &str = "text/css text/plain text/xml application/javascript application/json application/xml image/svg+xml font/ttf font/otf application/vnd.ms-fontobject";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    pub enabled: bool,
    pub max_size: String,
    pub inactive: String,
    pub valid: String,
    pub use_stale: bool,
    pub lock: bool,
    pub status_header: bool,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_size: "1g".into(),
            inactive: "60m".into(),
            valid: "10m".into(),
            use_stale: false,
            lock: true,
            status_header: true,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GzipConfig {
    pub enabled: bool,
    pub types: String,
    pub min_length: u32,
    pub comp_level: u32,
    pub vary: bool,
    pub proxied: bool,
}
impl Default for GzipConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            types: "text".into(),
            min_length: 1024,
            comp_level: 5,
            vary: true,
            proxied: true,
        }
    }
}

fn field_name(key: &str) -> String {
    let label = match key {
        "enabled" => "서버 기본 켜기",
        "maxSize" => "최대 저장 용량",
        "inactive" => "미사용 보관 기간",
        "valid" => "응답 유효기간",
        "useStale" => "오류 시 오래된 캐시 사용",
        "lock" => "동시 요청 잠금",
        "statusHeader" => "X-Cache-Status 응답 헤더",
        "types" => "압축 대상",
        "minLength" => "최소 응답 크기",
        "compLevel" => "압축 수준",
        "vary" => "Vary 응답 헤더",
        "proxied" => "프록시 요청도 압축",
        _ => return key.into(),
    };
    format!("{label}({key})")
}
fn boolean(obj: &Map<String, Value>, key: &str, fallback: bool, what: &str) -> Result<bool> {
    match obj.get(key) {
        None => Ok(fallback),
        Some(v) => v.as_bool().ok_or_else(|| {
            format!(
                "{what} 설정의 {} 값은 켜기/끄기(boolean)여야 합니다.",
                field_name(key)
            )
            .into()
        }),
    }
}
fn enum_value(
    obj: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
    fallback: &str,
    what: &str,
) -> Result<String> {
    let Some(v) = obj.get(key) else {
        return Ok(fallback.into());
    };
    v.as_str()
        .filter(|s| allowed.contains(s))
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "{what} 설정의 {} 값이 허용 목록({}) 밖입니다.",
                field_name(key),
                allowed.join(", ")
            )
            .into()
        })
}
fn integer(
    obj: &Map<String, Value>,
    key: &str,
    min: u32,
    max: u32,
    fallback: u32,
    what: &str,
) -> Result<u32> {
    let Some(v) = obj.get(key) else {
        return Ok(fallback);
    };
    v.as_f64()
        .filter(|v| v.fract() == 0.0 && *v >= min as f64 && *v <= max as f64)
        .map(|v| v as u32)
        .ok_or_else(|| {
            format!(
                "{what} 설정의 {} 값은 {min}-{max} 사이의 정수여야 합니다.",
                field_name(key)
            )
            .into()
        })
}
pub fn parse_cache_config(raw: &Value) -> Result<Option<CacheConfig>> {
    if raw.is_null() {
        return Ok(None);
    }
    let obj = raw
        .as_object()
        .ok_or_else(|| ControlError::from("캐시 설정 형식이 올바르지 않습니다."))?;
    Ok(Some(CacheConfig {
        enabled: boolean(obj, "enabled", false, "캐시")?,
        max_size: enum_value(obj, "maxSize", CACHE_MAX_SIZES, "1g", "캐시")?,
        inactive: enum_value(obj, "inactive", CACHE_INACTIVES, "60m", "캐시")?,
        valid: enum_value(obj, "valid", CACHE_VALIDS, "10m", "캐시")?,
        use_stale: boolean(obj, "useStale", false, "캐시")?,
        lock: boolean(obj, "lock", true, "캐시")?,
        status_header: boolean(obj, "statusHeader", true, "캐시")?,
    }))
}
pub fn parse_gzip_config(raw: &Value) -> Result<Option<GzipConfig>> {
    if raw.is_null() {
        return Ok(None);
    }
    let obj = raw
        .as_object()
        .ok_or_else(|| ControlError::from("gzip 설정 형식이 올바르지 않습니다."))?;
    Ok(Some(GzipConfig {
        enabled: boolean(obj, "enabled", false, "gzip")?,
        types: enum_value(obj, "types", GZIP_TYPES_PRESETS, "text", "gzip")?,
        min_length: integer(obj, "minLength", 0, GZIP_MIN_LENGTH_MAX, 1024, "gzip")?,
        comp_level: integer(
            obj,
            "compLevel",
            GZIP_COMP_LEVEL_MIN,
            GZIP_COMP_LEVEL_MAX,
            5,
            "gzip",
        )?,
        vary: boolean(obj, "vary", true, "gzip")?,
        proxied: boolean(obj, "proxied", true, "gzip")?,
    }))
}
pub fn parse_performance_override(raw: &Value) -> Result<Option<bool>> {
    if raw.is_null() {
        return Ok(None);
    }
    raw.as_bool().map(Some).ok_or_else(|| {
        "경로별 캐시/압축 설정은 「서버 설정 따름/켜기/끄기」 중 하나여야 합니다.".into()
    })
}
pub fn is_cache_effective(cache: Option<&CacheConfig>, override_value: Option<bool>) -> bool {
    override_value.unwrap_or_else(|| cache.is_some_and(|c| c.enabled))
}
pub fn is_gzip_effective(gzip: Option<&GzipConfig>, override_value: Option<bool>) -> bool {
    override_value.unwrap_or_else(|| gzip.is_some_and(|g| g.enabled))
}

fn explicit_buffering(list: &Value) -> Option<&str> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["name"] == "proxy_buffering")
        .filter_map(|d| d["args"][0].as_str().filter(|s| matches!(*s, "on" | "off")))
        .next_back()
}

/// Expects normalized performance objects and overrides, as in the original
/// service contract. The renderer parses all untrusted values before this check.
pub fn validate_performance_conflicts(cfg: &Value) -> Result<()> {
    if cfg.get("gzip").is_some_and(|v| !v.is_null() && v != false)
        && cfg["serverDirectives"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|d| d["name"] == "gzip")
    {
        return Err("gzip 설정과 고급 디렉티브의 gzip 이 함께 있습니다. 디렉티브 탭의 고급 설정에서 gzip 줄을 지우세요.".into());
    }
    let server_buffering = explicit_buffering(&cfg["serverDirectives"]);
    let server_enabled = cfg["cache"]["enabled"].as_bool().unwrap_or(false);
    let check = |path: &str, directives: &Value, override_value: &Value| -> Result<()> {
        if override_value.as_bool().unwrap_or(server_enabled)
            && explicit_buffering(directives).or(server_buffering) == Some("off")
        {
            return Err(format!("location {path}: proxy_buffering off 이면 캐시가 동작하지 않습니다. 이 경로의 캐시를 끄거나 proxy_buffering 을 지우세요.").into());
        }
        Ok(())
    };
    if let Some(locations) = cfg["locations"].as_array().filter(|a| !a.is_empty()) {
        for l in locations {
            check(
                l["path"].as_str().unwrap_or(""),
                &l["directives"],
                &l["cacheEnabled"],
            )?;
        }
    } else {
        check("/", &cfg["locationDirectives"], &Value::Null)?;
    }
    Ok(())
}
