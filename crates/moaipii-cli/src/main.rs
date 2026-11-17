//------------------------------------------------------------------
// mp.exe — moaipii 시험 도구 (텍스트 파일 하나 검사 · 버전 · 라벨 목록)
//=> DLL 과 같은 C 진입점(moaipii_*) 소스를 그대로 넣어 그 함수로 부른다. 그래서 mp 로 본 결과가 곧
//   moaipii.dll / libmoaipii.so 의 결과다(옵션 JSON 읽기·결과 JSON·해제까지 함께 시험된다).
//   명령:
//     version                    버전 · ABI · ko-pii 기준
//     labels                     지원 라벨 목록(JSON)
//     scan <파일|-> [옵션]       파일 바이트를 그대로 넘겨 검사 → 결과 JSON
//------------------------------------------------------------------

use std::ffi::{c_char, CStr, CString};
use std::io::Read;
use std::time::Instant;

use serde_json::{json, Value};

// DLL 과 똑같은 C 진입점 소스를 모듈로 넣는다 — mp 로 본 결과가 곧 DLL 의 결과가 되게.
#[allow(dead_code)]
#[path = "../../moaipii-ffi/src/lib.rs"]
mod ffi;

const USAGE: &str = "\
mp — moaipii 시험 도구

사용법:
  mp version
  mp labels
  mp scan <파일|-> [--labels A,B,…|all] [--offsets char|utf16|byte] [--values]
                   [--no-items] [--invalid-utf8 error|replace] [--today YYYY-MM-DD]
                   [--unknown-labels error|ignore] [--time]

  scan 은 파일 바이트를 그대로 넘긴다(UTF-8 이 아니면 --invalid-utf8 replace).
  결과 JSON 을 표준출력으로, --time 이면 걸린 시간을 표준오류로 낸다.
";

//------------------------------------------------------------------
// 고정 C 문자열 → String
//
// -in: p = 라이브러리가 준 고정 문자열(해제하지 않는 것)
//
// -out: String = 사본
// -out: error = 없음(UTF-8 이 아니면 대체 문자)
//------------------------------------------------------------------
fn cstr(p: *const c_char) -> String {
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

//------------------------------------------------------------------
// JSON 결과 함수를 불러 (코드, 문자열)로 받고 곧바로 해제
//
// -in: f = 결과 자리를 받아 코드를 돌려주는 호출
//
// -out: (i32, String) = 반환 코드와 결과 JSON 문자열
// -out: error = 없음
//------------------------------------------------------------------
fn call(f: impl FnOnce(*mut *mut c_char) -> i32) -> (i32, String) {
    let mut out: *mut c_char = std::ptr::null_mut();
    let code = f(&mut out);
    let s = if out.is_null() { String::new() } else { cstr(out) };
    ffi::moaipii_free(out);
    (code, s)
}

//------------------------------------------------------------------
// scan 명령
//=> 1) 옵션을 읽어 옵션 JSON 을 만든다(--labels all 이면 지원 라벨 전부)
//    2) 파일(또는 표준입력) 바이트를 그대로 moaipii_scan 에 넘긴다
//    3) 결과 JSON 을 출력하고, 실패면 종료코드 = 반환 코드
//
// -in: args = scan 뒤의 인자들
//
// -out: i32 = 종료 코드(0 = 성공)
// -out: error = 인자가 틀리면 사용법을 찍고 2
//------------------------------------------------------------------
fn cmd_scan(args: &[String]) -> i32 {
    let mut path: Option<&str> = None;
    let mut opts = serde_json::Map::new();
    let mut labels: Vec<String> = vec!["all".into()];
    let mut time = false;
    let mut i = 0;
    // 값이 필요한 옵션의 값을 꺼낸다
    let need = |i: &mut usize, name: &str| -> Option<String> {
        *i += 1;
        let v = args.get(*i).cloned();
        if v.is_none() { eprintln!("{} 뒤에 값이 필요합니다", name); }
        v
    };
    while i < args.len() {
        match args[i].as_str() {
            "--labels" => match need(&mut i, "--labels") { Some(v) => labels = v.split(',').map(|s| s.trim().to_string()).collect(), None => return 2 },
            "--offsets" => match need(&mut i, "--offsets") { Some(v) => { opts.insert("offsets".into(), json!(v)); } None => return 2 },
            "--invalid-utf8" => match need(&mut i, "--invalid-utf8") { Some(v) => { opts.insert("invalid_utf8".into(), json!(v)); } None => return 2 },
            "--today" => match need(&mut i, "--today") { Some(v) => { opts.insert("today".into(), json!(v)); } None => return 2 },
            "--unknown-labels" => match need(&mut i, "--unknown-labels") { Some(v) => { opts.insert("unknown_labels".into(), json!(v)); } None => return 2 },
            "--values" => { opts.insert("values".into(), json!(true)); }
            "--no-items" => { opts.insert("items".into(), json!(false)); }
            "--time" => time = true,
            a if a.starts_with("--") => { eprintln!("모르는 옵션: {}\n\n{}", a, USAGE); return 2; }
            a => {
                if path.is_some() { eprintln!("파일은 하나만 줍니다\n\n{}", USAGE); return 2; }
                path = Some(a);
            }
        }
        i += 1;
    }
    let Some(path) = path else { eprintln!("{}", USAGE); return 2; };
    // all = 지원 라벨 전부
    if labels.len() == 1 && labels[0] == "all" {
        let (_, s) = call(|o| ffi::moaipii_labels(o));
        let v: Value = serde_json::from_str(&s).unwrap_or(json!({}));
        labels = v["labels"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    }
    opts.insert("labels".into(), json!(labels));

    let mut bytes = Vec::new();
    let read = if path == "-" { std::io::stdin().read_to_end(&mut bytes).map(|_| ()) } else { std::fs::read(path).map(|b| bytes = b) };
    if let Err(e) = read { eprintln!("읽지 못했습니다 {}: {}", path, e); return 2; }

    let o = CString::new(Value::Object(opts).to_string()).expect("옵션에 NUL 없음");
    let t0 = Instant::now();
    let (code, s) = call(|out| ffi::moaipii_scan(bytes.as_ptr(), bytes.len(), o.as_ptr(), out));
    let dt = t0.elapsed();
    println!("{}", s);
    if time { eprintln!("{} 바이트 · {:.3}초", bytes.len(), dt.as_secs_f64()); }
    code
}

//------------------------------------------------------------------
// 진입점
//=> 첫 인자로 명령을 고른다.
//
// -in: 없음(명령줄 인자)
//
// -out: 없음(종료 코드로 끝낸다)
// -out: error = 모르는 명령이면 사용법과 종료코드 2
//------------------------------------------------------------------
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("version") => {
            println!("moaipii {} (abi {}, ko-pii 기준 {})",
                     cstr(ffi::moaipii_version()), ffi::moaipii_abi_version(), cstr(ffi::moaipii_kopii_basis()));
            0
        }
        Some("labels") => {
            let (code, s) = call(|o| ffi::moaipii_labels(o));
            println!("{}", s);
            code
        }
        Some("scan") => cmd_scan(&args[1..]),
        _ => { eprintln!("{}", USAGE); 2 }
    };
    std::process::exit(code);
}
