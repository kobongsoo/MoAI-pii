//------------------------------------------------------------------
// 감싸개 공통 시험 — Rust 실행기
//=> cases.json 의 사례를 Rust 감싸개(bindings/rust/moaipii_dyn.rs)로 돌려 한 줄에 한 사례씩 JSON 을 낸다.
//    1) 본문이 올바른 UTF-8 이면 감싸개의 scan(구조체 결과)을, 깨졌으면 scan_json(바이트)을 쓴다
//    2) values 가 있고 좌표가 이 언어의 자르기 단위(byte)면 &text[start..end] 가 값과 같은지 본다
//    3) threads 사례는 같은 MoaiPii 를 여러 스레드가 함께 써서 결과가 같은지 본다
//    4) fake_abi 는 MOAIPII_FAKE_LIB 를 불러 abi 오류가 나는지 본다
//
//   사용: runner <cases.json>   (라이브러리는 환경변수 MOAIPII_LIB)
//------------------------------------------------------------------

use std::collections::HashSet;
use std::path::Path;

use serde_json::{json, Value};

#[path = "../../../../bindings/rust/moaipii_dyn.rs"]
mod moaipii_dyn;
use moaipii_dyn::{MoaiPii, ScanOpts};

const LANG: &str = "rs";

//------------------------------------------------------------------
// base64 → 바이트 (표준 알파벳, 시험 파일만 읽으므로 작게 직접 구현)
//
// -in: s = base64 문자열
//
// -out: Vec<u8>
// -out: error = 없음(알파벳 밖 글자는 건너뛴다)
//------------------------------------------------------------------
fn b64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for &c in s.as_bytes() {
        if let Some(v) = val(c) {
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
    }
    out
}

//------------------------------------------------------------------
// 사례 한 번 검사 → 결과 JSON 조각(code·counts·items·unknown 또는 code·kind)
//
// -in: pii  = 불러온 라이브러리
// -in: data = 본문 바이트
// -in: c    = 사례
//
// -out: Value = 결과 조각
// -out: error = 없음
//------------------------------------------------------------------
fn once(pii: &MoaiPii, data: &[u8], c: &Value) -> Value {
    let o = &c["opts"];
    let labels: Vec<String> = c["labels"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    match std::str::from_utf8(data) {
        // 올바른 UTF-8 → 감싸개의 구조체 결과 경로
        Ok(text) => {
            let opts = ScanOpts {
                items: o["items"].as_bool().unwrap(),
                values: o["values"].as_bool().unwrap(),
                offsets: match o["offsets"].as_str().unwrap() { "utf16" => "utf16", "byte" => "byte", _ => "char" },
                unknown_ignore: o["unknown_labels"] == "ignore",
                today: o["today"].as_str().map(String::from),
            };
            let set: HashSet<String> = labels.into_iter().collect();
            match pii.scan(text, &set, &opts) {
                Ok(r) => {
                    let items: Vec<Value> = r.items.iter().map(|i| {
                        let mut m = json!({"label": i.label, "start": i.start, "end": i.end});
                        if let Some(v) = &i.value { m["value"] = json!(v); }
                        m
                    }).collect();
                    json!({"code": 0, "counts": r.counts, "items": items, "unknown": r.unknown})
                }
                Err(e) => json!({"code": if e.kind == "utf8" { 2 } else if e.kind == "arg" { 1 } else { 99 }, "kind": e.kind}),
            }
        }
        // 깨진 UTF-8 → 바이트를 그대로 넘기는 scan_json 경로
        Err(_) => {
            let mut opts = o.clone();
            opts["labels"] = json!(labels);
            let (code, v) = pii.scan_json(data, &opts.to_string());
            if code != 0 {
                json!({"code": code, "kind": v["error"]["kind"]})
            } else {
                json!({"code": 0, "counts": v["counts"], "items": v["items"], "unknown": v.get("unknown").cloned().unwrap_or(json!([]))})
            }
        }
    }
}

//------------------------------------------------------------------
// 진입점
//
// -in: 없음(명령줄: cases.json)
//
// -out: 없음(표준출력에 JSON 줄들)
// -out: error = 라이브러리를 못 불러오면 종료 코드 2
//------------------------------------------------------------------
fn main() {
    let path = std::env::args().nth(1).expect("cases.json 경로");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let pii = match MoaiPii::find_and_load(None, "MOAIPII_LIB") {
        Ok(p) => p,
        Err(e) => { eprintln!("{}", e); std::process::exit(2); }
    };
    for c in doc["cases"].as_array().unwrap() {
        if !c["langs"].as_array().unwrap().iter().any(|l| l == LANG) { continue; }
        let mut out = json!({"name": c["name"], "lang": LANG});
        if c["fake_abi"].as_bool() == Some(true) {
            let fake = std::env::var("MOAIPII_FAKE_LIB").unwrap();
            match MoaiPii::load(Path::new(&fake)) {
                Ok(_) => out["code"] = json!(0),
                Err(e) => {
                    out["code"] = json!(-1);
                    out["kind"] = json!(if e.contains("ABI") { "abi" } else { "load" });
                }
            }
            println!("{}", out);
            continue;
        }
        let rep = c["repeat"].as_u64().unwrap_or(1) as usize;
        let data = b64(c["text_b64"].as_str().unwrap()).repeat(rep);
        let first = once(&pii, &data, c);
        for (k, v) in first.as_object().unwrap() { out[k] = v.clone(); }
        // 이 언어의 자르기 단위(byte)면 원래 본문을 잘라 값과 맞춰 본다
        let o = &c["opts"];
        if first["code"] == 0 && o["values"] == true && o["offsets"] == "byte" {
            let ok = first["items"].as_array().unwrap().iter().all(|i| {
                let (s, e) = (i["start"].as_u64().unwrap() as usize, i["end"].as_u64().unwrap() as usize);
                data.get(s..e).map(|b| String::from_utf8_lossy(b).into_owned()) == i["value"].as_str().map(String::from)
            });
            out["slice_ok"] = json!(ok);
        }
        if let Some(n) = c["threads"].as_u64() {
            // 같은 MoaiPii 를 여러 스레드가 함께 쓴다(Sync)
            let res: Vec<Value> = std::thread::scope(|s| {
                let hs: Vec<_> = (0..n).map(|_| s.spawn(|| once(&pii, &data, c))).collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            out["threads_ok"] = json!(res.iter().all(|r| *r == first));
        }
        println!("{}", out);
    }
}
