//! moaipii C 진입점 — moaipii.dll / libmoaipii.so
//!
//! 설계: plan/MoAI-pii-개인정보검출-라이브러리분리-설계-20261117.html §4
//!
//! 다른 프로그램(Python ctypes · Rust libloading · Java JNA · PHP FFI · C++)이 이 라이브러리
//! 파일을 실행 중에 불러 쓴다. 경계에서는 (포인터, 길이) 바이트·C 문자열·정수만 오간다.
//!   · 본문은 (const uint8_t*, size_t) — 널 종료가 아니다(본문 속 \0 에서 잘리지 않게)
//!   · 옵션·결과 → UTF-8 JSON 문자열 (필드를 더해도 옛 감싸개가 깨지지 않게)
//!   · 라이브러리가 만든 char* → 반드시 moaipii_free 로 돌려받는다(0 으로 덮고 지운다)
//!   · Rust 패닉 → 경계 밖으로 새지 않게 잡아서 MP_ERR_PANIC 으로 바꾼다
//!   · 전역 상태 없음 → 여러 스레드에서 동시에 불러도 된다
//!   · 로그 없음 → 검출기가 로그를 남기면 개인정보가 샐 수 있다
//!
//! 헤더: include/moaipii.h (함수 모양을 바꾸면 ABI_VERSION 을 올리고 헤더도 같이 고친다)

use std::collections::HashSet;
use std::ffi::{c_char, c_int, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use moaipii as mp;
use serde_json::{json, Map, Value};

/// 함수 모양(이름·인자)이 바뀔 때만 올린다. 감싸개는 불러온 직후 이 값을 비교한다.
pub const ABI_VERSION: u32 = 1;

// 반환 코드 — moaipii.h 의 mp_status 와 같은 값이어야 한다.
pub const MP_OK: c_int = 0;
pub const MP_ERR_ARG: c_int = 1;
pub const MP_ERR_UTF8: c_int = 2;
pub const MP_ERR_PANIC: c_int = 99;

//------------------------------------------------------------------
// 패닉 내용을 사람이 읽는 문장으로
//=> catch_unwind 가 돌려준 상자에서 &str 이나 String 을 꺼낸다.
//
// -in: e = 패닉 값
//
// -out: String = 패닉 메시지(모르는 모양이면 "알 수 없는 패닉")
// -out: error = 없음
//------------------------------------------------------------------
fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "알 수 없는 패닉".to_string()
    }
}

//------------------------------------------------------------------
// 문자열을 C 쪽으로 내보내기
//=> CString 으로 만들어 소유권을 C 쪽에 넘긴다. 해제는 moaipii_free 가 한다.
//   결과는 serde_json 이 만든 JSON 이라 \0 이 \u0000 으로 이스케이프돼 있다. 혹시 남은
//   \0 은 C 문자열을 끊으므로 지운다.
//
// -in: out = 결과를 받을 자리(NULL 이면 아무 일도 하지 않음)
// -in: s   = 내보낼 문자열
//
// -out: 없음
// -out: error = 없음
//------------------------------------------------------------------
fn give(out: *mut *mut c_char, s: String) {
    if out.is_null() {
        return;
    }
    let c = CString::new(s).unwrap_or_else(|e| {
        let mut b = e.into_vec();
        b.retain(|&x| x != 0);
        CString::new(b).expect("NUL 을 지웠다")
    });
    // 안전: out 이 NULL 이 아님을 위에서 확인했다.
    unsafe { *out = c.into_raw() };
}

//------------------------------------------------------------------
// 오류 JSON 만들기
//
// -in: kind    = 오류 종류("arg"·"utf8"·"panic")
// -in: message = 사람이 읽는 사유
//
// -out: String = {"error":{"kind":…,"message":…}}
// -out: error = 없음
//------------------------------------------------------------------
fn err_json(kind: &str, message: &str) -> String {
    json!({"error": {"kind": kind, "message": message}}).to_string()
}

/// 본체가 내는 실패 — (반환 코드, 오류 종류, 사유)
type Fail = (c_int, &'static str, String);

//------------------------------------------------------------------
// 인자 오류 하나 만들기 (짧게 쓰려는 도우미)
//
// -in: why = 사유
//
// -out: Fail = (MP_ERR_ARG, "arg", 사유)
// -out: error = 없음
//------------------------------------------------------------------
fn arg_err(why: impl Into<String>) -> Fail {
    (MP_ERR_ARG, "arg", why.into())
}

//------------------------------------------------------------------
// JSON 결과를 내는 함수의 공통 틀
//=> 본체를 catch_unwind 로 감싸 패닉이 C 쪽으로 새지 않게 하고, 결과 JSON 을 out 에 적는다.
//    1) out 이 NULL 이면 MP_ERR_ARG (결과를 돌려줄 곳이 없다)
//    2) 본체가 Err((코드, 종류, 사유)) → 그 코드 + {"error":{…}}
//    3) 패닉 → MP_ERR_PANIC + {"error":{"kind":"panic"}}
//
// -in: out  = 결과 JSON 자리
// -in: body = 성공 JSON 문자열을 돌려주는 본체
//
// -out: c_int = 반환 코드(mp_status)
// -out: error = 없음(모든 실패를 코드로 바꾼다)
//------------------------------------------------------------------
fn guarded(out: *mut *mut c_char, body: impl FnOnce() -> Result<String, Fail>) -> c_int {
    if out.is_null() {
        return MP_ERR_ARG;
    }
    // 안전: 실패 때도 out 이 '쓰레기 포인터'로 남지 않게 먼저 비운다.
    unsafe { *out = std::ptr::null_mut() };
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(s)) => { give(out, s); MP_OK }
        Ok(Err((code, kind, why))) => { give(out, err_json(kind, &why)); code }
        Err(e) => { give(out, err_json("panic", &format!("moaipii 내부 오류: {}", panic_msg(e)))); MP_ERR_PANIC }
    }
}

//------------------------------------------------------------------
// ABI 버전
//=> 감싸개가 자기와 같은 함수 모양의 라이브러리인지 확인하는 데 쓴다.
//
// -in: 없음
//
// -out: u32 = ABI_VERSION
// -out: error = 없음
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_abi_version() -> u32 {
    ABI_VERSION
}

//------------------------------------------------------------------
// 라이브러리 버전 문자열
//=> "크레이트버전+git해시" (예: "0.1.0+abc1234"). 프로세스가 끝날 때까지 살아 있는
//   고정 문자열이라 부른 쪽은 해제하지 않는다.
//
// -in: 없음
//
// -out: const char* = 버전 문자열
// -out: error = 없음
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_version() -> *const c_char {
    static V: OnceLock<CString> = OnceLock::new();
    V.get_or_init(|| {
        CString::new(format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("MOAIPII_GIT"))).expect("NUL 없음")
    })
    .as_ptr()
}

//------------------------------------------------------------------
// 어느 ko-pii 를 보고 옮겼나
//=> 검출 기준이 된 ko-pii 판(예: "1.15.2"). 골든 시험이 정답표의 판과 같은지 지킨다.
//   고정 문자열이라 해제하지 않는다.
//
// -in: 없음
//
// -out: const char* = ko-pii 판 문자열
// -out: error = 없음
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_kopii_basis() -> *const c_char {
    static V: OnceLock<CString> = OnceLock::new();
    V.get_or_init(|| CString::new(mp::KOPII_PORTED_FROM).expect("NUL 없음")).as_ptr()
}

//------------------------------------------------------------------
// 지원 라벨 목록
//=> 규칙 편집기 같은 곳이 "켤 수 있는 라벨"을 보여 줄 때 쓴다.
//
// -in: out_json = 결과 JSON 자리 {"labels":[…], "aliases":{"PRESCRIPTION":"PRESCRIPTION_ID"}}
//
// -out: mp_status = MP_OK · MP_ERR_ARG(out_json 이 NULL) · MP_ERR_PANIC
// -out: error = 실패 사유는 결과 JSON 의 "error"
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_labels(out_json: *mut *mut c_char) -> c_int {
    guarded(out_json, || {
        let aliases: Map<String, Value> =
            mp::ALIASES.iter().map(|(a, r)| (a.to_string(), json!(r))).collect();
        Ok(json!({"labels": mp::LABELS, "aliases": aliases}).to_string())
    })
}

//------------------------------------------------------------------
// "YYYY-MM-DD" → (년, 월, 일)
//
// -in: s = 날짜 문자열
//
// -out: (년, 월, 일)
// -out: error = 모양이 틀리면 Err(사유)
//------------------------------------------------------------------
fn parse_day(s: &str) -> Result<(i64, u32, u32), String> {
    let p: Vec<&str> = s.split('-').collect();
    let bad = || format!("today 는 \"YYYY-MM-DD\" 여야 합니다: {}", s);
    if p.len() != 3 { return Err(bad()); }
    let y: i64 = p[0].parse().map_err(|_| bad())?;
    let m: u32 = p[1].parse().map_err(|_| bad())?;
    let d: u32 = p[2].parse().map_err(|_| bad())?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return Err(bad()); }
    Ok((y, m, d))
}

//------------------------------------------------------------------
// 풀린 검사 옵션
// -필드: labels          = 켤 라벨(아는 것만)
// -필드: unknown         = 모르는 라벨(unknown_labels:"ignore" 일 때 결과에 알려 준다)
// -필드: opts            = 본체에 넘길 옵션
// -필드: replace_invalid = 깨진 UTF-8 을 U+FFFD 로 고쳐 검사할지
//------------------------------------------------------------------
struct ScanReq {
    labels: HashSet<String>,
    unknown: Vec<String>,
    opts: mp::Opts,
    replace_invalid: bool,
}

//------------------------------------------------------------------
// 옵션 JSON 읽기
//=> 설계 §4-1 의 키를 읽는다. 빠진 키는 기본값, 모르는 키는 무시한다
//   (새 버전 감싸개의 옵션이 옛 DLL 에서 깨지지 않게).
//    1) labels(필수, 문자열 배열) — 모르는 라벨은 unknown_labels 가 "ignore" 가 아니면 오류
//    2) items(기본 true) · values(기본 false)
//    3) offsets: "char"(기본) | "utf16" | "byte"
//    4) invalid_utf8: "error"(기본) | "replace"
//    5) today: null | "YYYY-MM-DD"
//
// -in: s = 옵션 JSON 문자열
//
// -out: ScanReq = 풀린 옵션
// -out: error = 모양이 틀리면 Err(사유)
//------------------------------------------------------------------
fn parse_scan_opts(s: &str) -> Result<ScanReq, String> {
    let v: Value = serde_json::from_str(s.trim()).map_err(|e| format!("옵션 JSON 을 읽지 못했습니다: {}", e))?;
    let o = v.as_object().ok_or("옵션 JSON 은 객체({…})여야 합니다")?;
    // 켤 라벨 — 빈 배열은 허용(결과도 비어 있다), 문자열이 아닌 항목은 오류
    let arr = o.get("labels").and_then(Value::as_array).ok_or("옵션에 labels 배열이 없습니다")?;
    let mut names = Vec::with_capacity(arr.len());
    for x in arr {
        names.push(x.as_str().ok_or("labels 의 항목은 문자열이어야 합니다")?.to_string());
    }
    let ignore_unknown = match o.get("unknown_labels").and_then(Value::as_str).unwrap_or("error") {
        "error" => false,
        "ignore" => true,
        other => return Err(format!("unknown_labels 는 \"error\" 또는 \"ignore\" 입니다: {}", other)),
    };
    let unknown: Vec<String> = names.iter().filter(|n| !mp::is_known_label(n)).cloned().collect();
    if !unknown.is_empty() && !ignore_unknown {
        return Err(format!("모르는 라벨입니다: {} (moaipii_labels 로 목록을 볼 수 있습니다)", unknown.join(", ")));
    }
    // 참/거짓 옵션 하나 읽기(없거나 null 이면 기본값)
    let flag = |k: &str, def: bool| -> Result<bool, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(def),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{} 는 true/false 여야 합니다", k)),
        }
    };
    let offsets = match o.get("offsets").and_then(Value::as_str) {
        None => mp::Offsets::Char,
        Some(n) => mp::Offsets::parse(n).ok_or_else(|| format!("offsets 는 char·utf16·byte 중 하나입니다: {}", n))?,
    };
    let replace_invalid = match o.get("invalid_utf8").and_then(Value::as_str).unwrap_or("error") {
        "error" => false,
        "replace" => true,
        other => return Err(format!("invalid_utf8 는 \"error\" 또는 \"replace\" 입니다: {}", other)),
    };
    let today = match o.get("today") {
        None | Some(Value::Null) => None,
        Some(Value::String(d)) => Some(parse_day(d)?),
        Some(_) => return Err("today 는 null 또는 \"YYYY-MM-DD\" 입니다".to_string()),
    };
    Ok(ScanReq {
        // 모르는 라벨은 넘겨도 안 잡히지만, 굳이 넣지 않는다.
        labels: names.into_iter().filter(|n| mp::is_known_label(n)).collect(),
        unknown,
        opts: mp::Opts { items: flag("items", true)?, values: flag("values", false)?, offsets, today },
        replace_invalid,
    })
}

//------------------------------------------------------------------
// 본문 검사
//=> 본문을 (포인터, 길이)로 받아 개인정보 위치·건수를 JSON 으로 돌려준다.
//    1) 옵션 JSON 을 읽는다(labels 필수)
//    2) 본문 바이트 → UTF-8. 깨졌으면 invalid_utf8 에 따라 오류 또는 U+FFFD 로 고쳐 검사
//    3) moaipii::scan — 건수와 위치를 한 번의 스캔에서
//    4) byte 좌표 + 고친 본문이면 좌표를 '넘긴 원래 바이트'로 되돌린다
//
// -in: text      = 본문 바이트(UTF-8). len 이 0 이면 NULL 이어도 된다
// -in: len       = 본문 바이트 수
// -in: opts_json = 옵션 JSON(널 종료 UTF-8, 필수)
// -in: out_json  = 결과 JSON 자리 {"chars":…, "counts":{…}, "items":[…], "unknown":[…]?}
//
// -out: mp_status = MP_OK · MP_ERR_ARG · MP_ERR_UTF8 · MP_ERR_PANIC
// -out: error = 실패 사유는 결과 JSON 의 "error" {"kind": "arg"|"utf8"|"panic", "message"}
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_scan(text: *const u8, len: usize, opts_json: *const c_char,
                               out_json: *mut *mut c_char) -> c_int {
    guarded(out_json, || {
        if opts_json.is_null() {
            return Err(arg_err("opts_json 가 NULL 입니다 (labels 는 필수)"));
        }
        // 안전: 부른 쪽이 널 종료 문자열을 넘긴다는 것이 이 함수의 약속이다.
        let opts_s = unsafe { std::ffi::CStr::from_ptr(opts_json) }
            .to_str().map_err(|_| arg_err("opts_json 이 UTF-8 이 아닙니다"))?;
        let req = parse_scan_opts(opts_s).map_err(arg_err)?;
        if text.is_null() && len > 0 {
            return Err(arg_err("text 가 NULL 인데 len 이 0 이 아닙니다"));
        }
        // 안전: 부른 쪽이 len 바이트를 읽을 수 있는 버퍼를 넘긴다는 것이 약속이다.
        let bytes: &[u8] = if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(text, len) } };

        // 올바른 UTF-8 이면 그대로, 아니면 옵션에 따라 오류 또는 고쳐서
        let lossy;
        let (body, fix): (&str, Option<&mp::Lossy>) = match std::str::from_utf8(bytes) {
            Ok(s) => (s, None),
            Err(e) if !req.replace_invalid => {
                return Err((MP_ERR_UTF8, "utf8",
                    format!("본문이 UTF-8 이 아닙니다({}바이트째). invalid_utf8:\"replace\" 로 고쳐 검사할 수 있습니다",
                            e.valid_up_to())));
            }
            Err(_) => { lossy = mp::decode_lossy(bytes); (lossy.text.as_str(), Some(&lossy)) }
        };

        let r = mp::scan(body, &req.labels, &req.opts);
        // byte 좌표는 '넘긴 바이트' 기준이어야 substr 가 맞는다 — 고친 본문이면 되돌린다.
        let back = |b: usize| match (fix, req.opts.offsets) {
            (Some(l), mp::Offsets::Byte) => l.to_orig(b),
            _ => b,
        };
        let items: Vec<Value> = r.items.iter().map(|it| {
            let mut m = Map::new();
            m.insert("label".into(), json!(it.label));
            m.insert("start".into(), json!(back(it.start)));
            m.insert("end".into(), json!(back(it.end)));
            if let Some(v) = &it.value { m.insert("value".into(), json!(v)); }
            Value::Object(m)
        }).collect();
        let mut out = Map::new();
        out.insert("chars".into(), json!(r.chars));
        out.insert("counts".into(), json!(r.counts));
        out.insert("items".into(), Value::Array(items));
        if !req.unknown.is_empty() { out.insert("unknown".into(), json!(req.unknown)); }
        Ok(Value::Object(out).to_string())
    })
}

//------------------------------------------------------------------
// 라이브러리가 준 문자열 돌려받기
//=> 결과 JSON 에는 개인정보 위치·값이 들어 있을 수 있어, 해제하기 전에 0 으로 덮는다.
//   NULL 은 무시한다. 같은 포인터를 두 번 넘기면 안 된다.
//
// -in: p = 돌려줄 문자열
//
// -out: 없음
// -out: error = 없음
//------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn moaipii_free(p: *mut c_char) {
    if p.is_null() {
        return;
    }
    let _ = catch_unwind(|| {
        // 안전: give() 가 CString::into_raw 로 만든 포인터만 들어온다는 것이 약속이다.
        let mut b = unsafe { CString::from_raw(p) }.into_bytes_with_nul();
        // 최적화로 지우기가 빠지지 않게 volatile 로 쓴다.
        for x in b.iter_mut() {
            unsafe { std::ptr::write_volatile(x, 0) };
        }
        drop(b);
    });
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    //------------------------------------------------------------------
    // 시험 도우미: moaipii_scan 을 불러 (코드, JSON) 으로 받기
    //
    // -in: text = 본문 바이트
    // -in: opts = 옵션 JSON
    //
    // -out: (c_int, Value) = 반환 코드와 결과 JSON
    // -out: error = JSON 이 아니면 패닉(시험 실패)
    //------------------------------------------------------------------
    fn call(text: &[u8], opts: &str) -> (c_int, Value) {
        let o = CString::new(opts).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let code = moaipii_scan(text.as_ptr(), text.len(), o.as_ptr(), &mut out);
        let s = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        moaipii_free(out);
        (code, serde_json::from_str(&s).unwrap())
    }

    #[test]
    fn 본문의_NUL_뒤도_검사한다() {
        let mut t = b"head\0".to_vec();
        t.extend_from_slice("연락처 010-1234-5678".as_bytes());
        let (c, v) = call(&t, r#"{"labels":["PHONE"]}"#);
        assert_eq!(c, MP_OK, "{}", v);
        assert_eq!(v["counts"]["PHONE"], 1);
    }

    #[test]
    fn 깨진_UTF8_은_오류_또는_원래바이트_좌표() {
        let mut t = vec![0xFFu8, b' '];
        t.extend_from_slice("전화 010-1234-5678".as_bytes());
        let (c, v) = call(&t, r#"{"labels":["PHONE"]}"#);
        assert_eq!((c, v["error"]["kind"].as_str()), (MP_ERR_UTF8, Some("utf8")));
        let (c, v) = call(&t, r#"{"labels":["PHONE"],"invalid_utf8":"replace","offsets":"byte"}"#);
        assert_eq!(c, MP_OK);
        let (s, e) = (v["items"][0]["start"].as_u64().unwrap() as usize, v["items"][0]["end"].as_u64().unwrap() as usize);
        assert_eq!(&t[s..e], b"010-1234-5678");
    }

    #[test]
    fn 모르는_라벨과_빈_본문() {
        let (c, v) = call(b"x", r#"{"labels":["NOPE"]}"#);
        assert_eq!((c, v["error"]["kind"].as_str()), (MP_ERR_ARG, Some("arg")));
        let (c, v) = call(b"x", r#"{"labels":["NOPE","PHONE"],"unknown_labels":"ignore"}"#);
        assert_eq!(c, MP_OK);
        assert_eq!(v["unknown"], json!(["NOPE"]));
        let (c, v) = call(b"", r#"{"labels":["PHONE"]}"#);
        assert_eq!(c, MP_OK);
        assert_eq!(v["items"], json!([]));
        // NULL 본문 + 길이 0 은 빈 본문이다
        let o = CString::new(r#"{"labels":["PHONE"]}"#).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        assert_eq!(moaipii_scan(std::ptr::null(), 0, o.as_ptr(), &mut out), MP_OK);
        moaipii_free(out);
        assert_eq!(moaipii_scan(std::ptr::null(), 3, o.as_ptr(), &mut out), MP_ERR_ARG);
        moaipii_free(out);
        // 결과 자리가 NULL 이면 ARG
        assert_eq!(moaipii_scan(b"x".as_ptr(), 1, o.as_ptr(), std::ptr::null_mut()), MP_ERR_ARG);
    }

    #[test]
    fn 옵션_모양_오류() {
        for bad in [r#"[]"#, r#"{}"#, r#"{"labels":[1]}"#, r#"{"labels":[],"offsets":"x"}"#,
                    r#"{"labels":[],"items":"yes"}"#, r#"{"labels":[],"today":"2026/1/1"}"#] {
            let (c, _) = call(b"x", bad);
            assert_eq!(c, MP_ERR_ARG, "{}", bad);
        }
    }

    #[test]
    fn 값은_요청할_때만_싣는다() {
        let t = "메일 a.b@example.co.kr".as_bytes();
        let (_, v) = call(t, r#"{"labels":["EMAIL"]}"#);
        assert!(v["items"][0].get("value").is_none());
        let (_, v) = call(t, r#"{"labels":["EMAIL"],"values":true}"#);
        assert_eq!(v["items"][0]["value"], "a.b@example.co.kr");
        let (_, v) = call(t, r#"{"labels":["EMAIL"],"items":false}"#);
        assert_eq!(v["items"], json!([]));
        assert_eq!(v["counts"]["EMAIL"], 1);
    }

    #[test]
    fn 작은_함수들() {
        assert_eq!(moaipii_abi_version(), ABI_VERSION);
        let v = unsafe { CStr::from_ptr(moaipii_version()) }.to_str().unwrap();
        assert!(v.starts_with(env!("CARGO_PKG_VERSION")));
        let k = unsafe { CStr::from_ptr(moaipii_kopii_basis()) }.to_str().unwrap();
        assert_eq!(k, mp::KOPII_PORTED_FROM);
        let mut out: *mut c_char = std::ptr::null_mut();
        assert_eq!(moaipii_labels(&mut out), MP_OK);
        let s = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        moaipii_free(out);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["labels"].as_array().unwrap().len(), 20);
        assert_eq!(v["aliases"]["PRESCRIPTION"], "PRESCRIPTION_ID");
        moaipii_free(std::ptr::null_mut());
    }
}
