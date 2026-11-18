//------------------------------------------------------------------
// moaipii-test(.exe) — 배포된 DLL 을 실제로 불러 시험하는 CLI
//=> mp.exe 는 C 진입점 소스를 '안에 넣어' 만든 도구라 DLL 파일을 열지 않는다. 이 도구는
//   배포 묶음의 moaipii.dll / libmoaipii.so 파일을 실행 중에 불러(Rust 감싸개 moaipii_dyn.rs)
//   연동하는 프로그램과 똑같은 길로 시험한다. 그래서 "이 PC·서버에서 DLL 이 열리는가,
//   판(ABI)이 맞는가, 검출이 기대대로인가"를 DLL 을 받은 쪽이 바로 확인할 수 있다.
//   명령:
//     info · labels · demo · selftest · scan · mask · bench · version · help
//   사용법 전체는 USAGE, 자세한 설명은 doc/moaipii-test-guide.html.
//------------------------------------------------------------------

#[allow(dead_code)]
#[path = "../../../bindings/rust/moaipii_dyn.rs"]
mod moaipii_dyn;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use moaipii_dyn::MoaiPii;
use serde_json::{json, Value};

// 표준출력에 한 줄 쓰기 — `| head` 처럼 읽는 쪽이 먼저 닫혀도 패닉하지 않게 쓰기 오류는 버린다.
macro_rules! out {
    () => {{ let _ = writeln!(std::io::stdout()); }};
    ($($t:tt)*) => {{ let _ = writeln!(std::io::stdout(), $($t)*); }};
}

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("MOAIPII_GIT"));

const USAGE: &str = "\
moaipii-test — moaipii 라이브러리(DLL/.so) 시험 도구

사용법:
  moaipii-test [--lib <파일|폴더>] <명령> [옵션]

명령:
  info                         불러온 라이브러리 경로·버전·ABI·라벨 수
  labels [--json]              지원 라벨 목록
  demo                         라벨 20개를 예문 하나씩으로 검사 (PASS/FAIL)
  selftest [--today YYYY-MM-DD] demo + 좌표 단위·옵션·오류·스레드 시험 (연동 전 점검)
  scan <파일|-> | --text <문장> 검사 결과를 표로
       [--labels A,B|all] [--offsets char|utf16|byte] [--no-items] [--hide-values]
       [--ignore-unknown] [--invalid-utf8 error|replace] [--today YYYY-MM-DD]
       [--json [--values] [--pretty]]   DLL 결과 JSON 그대로(한 줄, 실패도 error JSON)
       [--opts '<옵션 JSON>' | --opts @파일]  옵션 JSON 을 DLL 에 그대로 넘김(--labels 등 무시)
       [--show-opts]                     넘긴 옵션 JSON 을 표준오류로
  mask <파일|-> | --text <문장> 검출 자리를 가린 본문 출력
       [--labels ...] [--style star|label|keep] [--keep N] [--out <파일>] [--invalid-utf8 ...]
  bench <파일|-> | --text <문장> 처리 속도 측정
       [--labels ...] [--repeat N] [--threads N]
  version                      이 도구의 버전
  help                         이 도움말

라이브러리 찾는 순서: --lib → 환경변수 MOAIPII_LIB → 이 실행파일 폴더 → 그 상위 폴더
종료 코드: 0 성공 · 1 시험 실패 · 2 인자 오류 · 3 라이브러리 불러오기 실패 · 4 검사 오류
";

// 라벨별 예문 — 번호는 모두 체크섬만 맞춘 가짜값이다(실제 개인정보 아님).
// (라벨, 예문, 잡혀야 할 값)
const DEMO: &[(&str, &str, &str)] = &[
    ("RRN", "주민등록번호 951006-1068003 입니다.", "951006-1068003"),
    ("FRN", "외국인등록번호 910410-5783619 확인.", "910410-5783619"),
    ("BUSINESS_REG", "사업자등록번호 690-61-68110 입니다.", "690-61-68110"),
    ("CORP_REG", "법인등록번호 165705-5641120 등기.", "165705-5641120"),
    ("DRIVER_LICENSE", "운전면허번호 14-54-881192-36 입니다.", "14-54-881192-36"),
    ("PASSPORT", "여권번호 M12345678 소지.", "M12345678"),
    ("CARD", "카드번호 4548-8365-5205-6218 로 결제.", "4548-8365-5205-6218"),
    ("MEDICAL_INSURANCE", "건강보험 증번호 1-2345678901 확인.", "1-2345678901"),
    ("PRESCRIPTION_ID", "처방전 교부번호 202401150001 입니다.", "202401150001"),
    ("PNU", "필지고유번호(PNU) 1168010100100010000 조회.", "1168010100100010000"),
    ("PHONE", "연락처 010-1234-5678 로 주세요.", "010-1234-5678"),
    ("EMAIL", "메일 hong@example.co.kr 로 회신.", "hong@example.co.kr"),
    ("IP", "접속 IP 192.168.10.25 를 기록.", "192.168.10.25"),
    ("VEHICLE", "차량 12가3456 주차.", "12가3456"),
    ("URL", "자료는 https://example.co.kr/doc/1 참고.", "https://example.co.kr/doc/1"),
    ("ADDRESS", "주소는 서울특별시 강남구 테헤란로 152 입니다.", "서울특별시 강남구 테헤란로 152"),
    ("NATIONALITY", "국적은 베트남 입니다.", "베트남"),
    ("ACCOUNT", "계좌번호 110-234-567890 로 입금.", "110-234-567890"),
    ("EDI_DRUG", "EDI 코드 641900010 으로 청구.", "641900010"),
    ("COURT_CASE", "사건번호 2023가합12345 판결문.", "2023가합12345"),
];

//------------------------------------------------------------------
// 명령줄 인자 묶음
// -필드: cmd      = 명령 이름(info, scan …)
// -필드: file     = 검사할 파일 경로("-" = 표준입력)
// -필드: text     = --text 로 준 문장(파일 대신)
// -필드: lib      = --lib 로 준 라이브러리 파일·폴더
// -필드: labels   = 켤 라벨("all" 또는 "RRN,PHONE")
// -필드: offsets  = 좌표 단위 char | utf16 | byte
// -필드: no_items / hide_values / ignore_unknown / json / values = 켜고 끄는 옵션
// -필드: pretty    = --json 결과를 들여쓰기해서 출력
// -필드: show_opts = DLL 에 넘긴 옵션 JSON 을 표준오류로 보여 줌
// -필드: opts_json = --opts 로 준 옵션 JSON(또는 @파일) — 주면 DLL 에 그대로 넘긴다
// -필드: invalid_utf8 = error | replace
// -필드: today    = 미래 생년월일 기준일
// -필드: style · keep · out = mask 옵션
// -필드: repeat · threads   = bench 옵션
//------------------------------------------------------------------
#[derive(Default)]
struct Args {
    cmd: String,
    file: Option<String>,
    text: Option<String>,
    lib: Option<String>,
    labels: String,
    offsets: String,
    no_items: bool,
    hide_values: bool,
    ignore_unknown: bool,
    json: bool,
    values: bool,
    pretty: bool,
    show_opts: bool,
    opts_json: Option<String>,
    invalid_utf8: String,
    today: Option<String>,
    style: String,
    keep: usize,
    out: Option<String>,
    repeat: usize,
    threads: usize,
}

//------------------------------------------------------------------
// 명령줄 해석
//=> 옵션은 어느 자리에 와도 된다. 옵션이 아닌 첫 낱말이 명령, 둘째가 파일이다.
//
// -in: argv = 프로그램 이름을 뺀 인자들
//
// -out: Args = 해석 결과(기본값 채움)
// -out: error = 모르는 옵션·값 빠짐·숫자 아님이면 Err(사유)
//------------------------------------------------------------------
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        labels: "all".into(), offsets: "char".into(), invalid_utf8: "error".into(),
        style: "star".into(), keep: 3, repeat: 20, threads: 1, ..Default::default()
    };
    let mut i = 0;
    // 값이 필요한 옵션의 값을 꺼낸다
    let take = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        argv.get(*i).cloned().ok_or_else(|| format!("{} 뒤에 값이 필요합니다", name))
    };
    // 숫자 값 옵션
    let num = |s: String, name: &str| -> Result<usize, String> {
        s.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| format!("{} 는 1 이상의 숫자여야 합니다: {}", name, s))
    };
    while i < argv.len() {
        let s = argv[i].as_str();
        match s {
            "--lib" => a.lib = Some(take(&mut i, s)?),
            "--text" => a.text = Some(take(&mut i, s)?),
            "--labels" => a.labels = take(&mut i, s)?,
            "--offsets" => {
                a.offsets = take(&mut i, s)?;
                if !["char", "utf16", "byte"].contains(&a.offsets.as_str()) {
                    return Err(format!("--offsets 는 char · utf16 · byte 중 하나입니다: {}", a.offsets));
                }
            }
            "--invalid-utf8" => {
                a.invalid_utf8 = take(&mut i, s)?;
                if !["error", "replace"].contains(&a.invalid_utf8.as_str()) {
                    return Err(format!("--invalid-utf8 는 error · replace 중 하나입니다: {}", a.invalid_utf8));
                }
            }
            "--today" => a.today = Some(take(&mut i, s)?),
            "--style" => {
                a.style = take(&mut i, s)?;
                if !["star", "label", "keep"].contains(&a.style.as_str()) {
                    return Err(format!("--style 은 star · label · keep 중 하나입니다: {}", a.style));
                }
            }
            "--keep" => a.keep = num(take(&mut i, s)?, s)?,
            "--out" => a.out = Some(take(&mut i, s)?),
            "--repeat" => a.repeat = num(take(&mut i, s)?, s)?,
            "--threads" => a.threads = num(take(&mut i, s)?, s)?,
            "--no-items" => a.no_items = true,
            "--hide-values" => a.hide_values = true,
            "--ignore-unknown" => a.ignore_unknown = true,
            "--json" => a.json = true,
            "--values" => a.values = true,
            "--pretty" => a.pretty = true,
            "--show-opts" => a.show_opts = true,
            "--opts" => a.opts_json = Some(take(&mut i, s)?),
            "-h" | "--help" => a.cmd = "help".into(),
            "-V" | "--version" => a.cmd = "version".into(),
            // "-" 하나는 표준입력이라는 뜻의 파일 자리다
            x if x.starts_with("--") => return Err(format!("모르는 옵션: {}", x)),
            x => {
                if a.cmd.is_empty() { a.cmd = x.to_string(); }
                else if a.file.is_none() { a.file = Some(x.to_string()); }
                else { return Err(format!("인자가 너무 많습니다: {}", x)); }
            }
        }
        i += 1;
    }
    if a.cmd.is_empty() { a.cmd = "help".into(); }
    Ok(a)
}

//------------------------------------------------------------------
// 라이브러리 불러오기
//=> --lib 가 있으면 그것만, 없으면 MOAIPII_LIB → 실행파일 폴더 → 상위 폴더 순으로 찾는다.
//
// -in: a = 해석한 인자(lib)
//
// -out: MoaiPii = 불러온 라이브러리
// -out: error = 못 찾음·못 엶·ABI 다름이면 Err(찾아본 곳을 포함한 사유)
//------------------------------------------------------------------
fn load(a: &Args) -> Result<MoaiPii, String> {
    MoaiPii::find_and_load(a.lib.as_deref().map(Path::new), "MOAIPII_LIB")
}

//------------------------------------------------------------------
// 검사할 본문 바이트 읽기
//=> --text 가 있으면 그 문장, 아니면 파일(또는 "-" = 표준입력)을 바이트 그대로 읽는다.
//   인코딩을 바꾸지 않는다 — UTF-8 이 아니면 --invalid-utf8 replace 로 넘긴다.
//
// -in: a = 해석한 인자(text, file)
//
// -out: Vec<u8> = 본문 바이트
// -out: error = 둘 다 없거나 읽기 실패면 Err(사유)
//------------------------------------------------------------------
fn read_input(a: &Args) -> Result<Vec<u8>, String> {
    if let Some(t) = &a.text { return Ok(t.as_bytes().to_vec()); }
    match a.file.as_deref() {
        None => Err("검사할 파일 또는 --text 를 주세요".into()),
        Some("-") => {
            let mut b = Vec::new();
            std::io::stdin().read_to_end(&mut b).map_err(|e| format!("표준입력을 읽지 못했습니다: {}", e))?;
            Ok(b)
        }
        Some(p) => std::fs::read(p).map_err(|e| format!("읽지 못했습니다 {}: {}", p, e)),
    }
}

//------------------------------------------------------------------
// --labels 값 → 라벨 목록
//=> "all" 이면 라이브러리가 지원한다고 알려 준 라벨 전부, 아니면 쉼표로 나눠 대문자로.
//
// -in: pii  = 불러온 라이브러리
// -in: spec = "all" 또는 "RRN,PHONE"
//
// -out: Vec<String> = 켤 라벨
// -out: error = 라벨 목록 요청이 실패하면 Err
//------------------------------------------------------------------
fn parse_labels(pii: &MoaiPii, spec: &str) -> Result<Vec<String>, String> {
    if spec.eq_ignore_ascii_case("all") {
        return pii.labels().map_err(|e| e.to_string());
    }
    Ok(spec.split(',').map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect())
}

//------------------------------------------------------------------
// 옵션 JSON 으로 검사 (바이트 본문)
//=> 감싸개의 scan_json 을 그대로 부른다. 깨진 UTF-8 본문도 넘길 수 있게 바이트로 다룬다.
//
// -in: pii  = 불러온 라이브러리
// -in: data = 본문 바이트
// -in: opts = 옵션 JSON(labels 필수)
//
// -out: (i32, Value) = 반환 코드와 결과 JSON
// -out: error = 없음(실패는 코드와 결과의 error 로)
//------------------------------------------------------------------
fn raw(pii: &MoaiPii, data: &[u8], opts: &Value) -> (i32, Value) {
    pii.scan_json(data, &opts.to_string())
}

//------------------------------------------------------------------
// 라이브러리 오류 한 줄 출력
//
// -in: code = 반환 코드
// -in: v    = 결과 JSON({"error":{kind,message}})
//
// -out: i32 = 이 도구의 종료 코드 4
// -out: error = 없음
//------------------------------------------------------------------
fn lib_error(code: i32, v: &Value) -> i32 {
    eprintln!("[moaipii 오류] kind={} code={} : {}", v["error"]["kind"].as_str().unwrap_or("?"), code,
              v["error"]["message"].as_str().unwrap_or("알 수 없는 오류"));
    4
}

//------------------------------------------------------------------
// 값 가리기(화면 출력용) — 앞 2글자만 남기고 *
//
// -in: v = 원래 값
//
// -out: String = 가린 값
// -out: error = 없음
//------------------------------------------------------------------
fn redact(v: &str) -> String {
    let n = v.chars().count();
    v.chars().take(2).collect::<String>() + &"*".repeat(n.saturating_sub(2))
}

//------------------------------------------------------------------
// info 명령 — 불러온 라이브러리 정보
//
// -in: pii = 불러온 라이브러리
//
// -out: i32 = 종료 코드
// -out: error = 라벨 목록 실패면 4
//------------------------------------------------------------------
fn cmd_info(pii: &MoaiPii) -> i32 {
    let labels = match pii.labels() { Ok(l) => l, Err(e) => { eprintln!("[moaipii 오류] {}", e); return 4; } };
    out!("시험 도구    : moaipii-test {}", VERSION);
    out!("라이브러리   : {}", pii.path().display());
    out!("버전         : {}", pii.version());
    out!("ABI          : {}", moaipii_dyn::ABI_VERSION);
    out!("ko-pii 기준  : {}", pii.kopii_basis());
    out!("라벨         : {}개 (+ 별칭 PRESCRIPTION→PRESCRIPTION_ID)", labels.len());
    // 시험 도구와 DLL 이 다른 빌드면 알려 준다(같은 묶음에서 꺼냈는지 확인용).
    if pii.version() != VERSION {
        out!("[참고] 시험 도구와 라이브러리의 빌드가 다릅니다 — 같은 묶음의 파일인지 확인하세요");
    }
    0
}

//------------------------------------------------------------------
// labels 명령 — 지원 라벨 목록
//
// -in: pii = 불러온 라이브러리
// -in: a   = 인자(json 이면 원본 JSON)
//
// -out: i32 = 종료 코드
// -out: error = 라벨 목록 실패면 4
//------------------------------------------------------------------
fn cmd_labels(pii: &MoaiPii, a: &Args) -> i32 {
    let labels = match pii.labels() { Ok(l) => l, Err(e) => { eprintln!("[moaipii 오류] {}", e); return 4; } };
    if a.json {
        out!("{}", json!({"labels": labels, "aliases": {"PRESCRIPTION": "PRESCRIPTION_ID"}}));
    } else {
        for l in &labels { out!("{}", l); }
        out!("PRESCRIPTION  (별칭 → PRESCRIPTION_ID, 건수만 셈)");
    }
    0
}

//------------------------------------------------------------------
// scan 이 DLL 에 넘길 옵션 JSON 문자열 만들기
//=> --opts 를 주면 그 문자열(또는 @파일 내용)을 손대지 않고 그대로 쓴다 — DLL 의 옵션 해석을
//   직접 시험하려는 것이므로 --labels 등 다른 옵션은 무시한다.
//   안 주면 명령줄 옵션으로 만든다. 표로 낼 때는 값도 받아 보여 주고, --json 은 --values 를
//   줄 때만 값을 싣는다(라이브러리 기본과 같게).
//
// -in: pii = 불러온 라이브러리(--labels all 풀이용)
// -in: a   = 인자
//
// -out: (String, usize) = (옵션 JSON 문자열, 켠 라벨 수 — 모르면 0)
// -out: error = @파일을 못 읽거나 라벨 목록 요청이 실패하면 Err(사유)
//------------------------------------------------------------------
fn scan_opts(pii: &MoaiPii, a: &Args) -> Result<(String, usize), String> {
    if let Some(o) = &a.opts_json {
        let s = match o.strip_prefix('@') {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("옵션 파일을 읽지 못했습니다 {}: {}", p, e))?,
            None => o.clone(),
        };
        // 라벨 수는 표 머리줄에만 쓰므로, JSON 이 틀려도 여기서 막지 않고 DLL 이 판단하게 둔다
        let n = serde_json::from_str::<Value>(&s).ok()
            .and_then(|v| v["labels"].as_array().map(Vec::len)).unwrap_or(0);
        return Ok((s, n));
    }
    let labels = parse_labels(pii, &a.labels)?;
    let opts = json!({
        "labels": labels, "items": !a.no_items, "values": !a.json || a.values, "offsets": a.offsets,
        "invalid_utf8": a.invalid_utf8, "unknown_labels": if a.ignore_unknown { "ignore" } else { "error" },
        "today": a.today,
    });
    Ok((opts.to_string(), labels.len()))
}

//------------------------------------------------------------------
// scan 명령 — 검사 결과를 표(또는 DLL 결과 JSON 그대로)로
//=> 1) 본문을 바이트로 읽고 옵션 JSON 을 정한다(scan_opts)
//    2) moaipii_scan 을 부른다
//    3) --json 이면 DLL 이 준 결과 JSON 을 그대로 한 줄로(실패여도 error JSON 그대로),
//       아니면 건수(많은 순) · 위치(문서 순) 표
//
// -in: pii = 불러온 라이브러리
// -in: a   = 인자
//
// -out: i32 = 0 성공(0건 포함) · 2 입력 오류 · 4 검사 오류
// -out: error = 위 종료 코드로 알림
//------------------------------------------------------------------
fn cmd_scan(pii: &MoaiPii, a: &Args) -> i32 {
    let data = match read_input(a) { Ok(d) => d, Err(e) => { eprintln!("{}", e); return 2; } };
    let (opts, n_labels) = match scan_opts(pii, a) {
        Ok(x) => x,
        Err(e) => { eprintln!("{}", e); return if a.opts_json.is_some() { 2 } else { 4 }; }
    };
    if a.show_opts { eprintln!("[옵션 JSON] {}", opts); }
    let t0 = Instant::now();
    let (code, v) = pii.scan_json(&data, &opts);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if a.json {
        // DLL 결과는 키가 정렬된 한 줄 JSON 이고, 같은 규칙으로 다시 적으면 바이트까지 같다.
        // 실패도 DLL 이 준 {"error":{...}} 를 그대로 내고 종료 코드로 알린다.
        let s = if a.pretty { serde_json::to_string_pretty(&v).unwrap_or_default() } else { v.to_string() };
        out!("{}", s);
        return if code == 0 { 0 } else { 4 };
    }
    if code != 0 { return lib_error(code, &v); }
    // 실제로 넘긴 옵션의 좌표 단위(--opts 로 줬을 수도 있어 옵션 JSON 에서 읽는다, 없으면 DLL 기본 char)
    let unit = serde_json::from_str::<Value>(&opts).ok()
        .and_then(|o| o["offsets"].as_str().map(String::from)).unwrap_or_else(|| "char".into());
    out!("본문 {}자 ({} 바이트) · 라벨 {}개 · {:.2} ms · 좌표 단위 {}",
             v["chars"], data.len(), n_labels, ms, unit);
    if let Some(u) = v["unknown"].as_array().filter(|u| !u.is_empty()) {
        out!("모르는 라벨(무시함): {}", u.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
    }
    let counts = v["counts"].as_object().cloned().unwrap_or_default();
    if counts.is_empty() {
        out!("검출 없음");
        return 0;
    }
    // 건수 많은 순, 같으면 이름 순
    let mut cs: Vec<(String, u64)> = counts.iter().map(|(k, n)| (k.clone(), n.as_u64().unwrap_or(0))).collect();
    cs.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    out!("\n[건수] 합계 {}", cs.iter().map(|c| c.1).sum::<u64>());
    for (k, n) in &cs { out!("  {:<18} {}", k, n); }
    if a.no_items { return 0; }
    out!("\n[위치]");
    out!("  {:<18} {:>8} {:>8}  값", "라벨", "start", "end");
    for it in v["items"].as_array().cloned().unwrap_or_default() {
        let val = it["value"].as_str().unwrap_or("");
        // 줄바꿈이 낀 값(주소·계좌)은 한 줄로 보이게 바꿔 찍는다
        let shown = if a.hide_values { redact(val) } else { val.to_string() }.replace(['\r', '\n'], "⏎");
        out!("  {:<18} {:>8} {:>8}  {}", it["label"].as_str().unwrap_or(""),
                 it["start"].as_u64().unwrap_or(0), it["end"].as_u64().unwrap_or(0), shown);
    }
    0
}

//------------------------------------------------------------------
// mask 명령 — 검출 자리를 가린 본문 출력
//=> byte 좌표로 받아 원래 바이트에서 바로 바꾼다(그래서 깨진 UTF-8 본문도 그대로 보존).
//   뒤에서부터 바꿔야 앞쪽 좌표가 밀리지 않는다.
//    style=star  : 같은 글자 수의 *
//    style=label : [PHONE] 처럼 라벨 이름
//    style=keep  : 앞 --keep 글자만 남기고 *
//
// -in: pii = 불러온 라이브러리
// -in: a   = 인자
//
// -out: i32 = 0 성공 · 2 입출력 오류 · 4 검사 오류
// -out: error = 위 종료 코드로 알림
//------------------------------------------------------------------
fn cmd_mask(pii: &MoaiPii, a: &Args) -> i32 {
    let mut data = match read_input(a) { Ok(d) => d, Err(e) => { eprintln!("{}", e); return 2; } };
    let labels = match parse_labels(pii, &a.labels) { Ok(l) => l, Err(e) => { eprintln!("[moaipii 오류] {}", e); return 4; } };
    let opts = json!({"labels": labels, "offsets": "byte", "invalid_utf8": a.invalid_utf8});
    let (code, v) = raw(pii, &data, &opts);
    if code != 0 { return lib_error(code, &v); }
    let items = v["items"].as_array().cloned().unwrap_or_default();
    for it in items.iter().rev() {
        let s = it["start"].as_u64().unwrap_or(0) as usize;
        let e = (it["end"].as_u64().unwrap_or(0) as usize).min(data.len());
        if s >= e { continue; }
        let orig = String::from_utf8_lossy(&data[s..e]).into_owned();
        let n = orig.chars().count();
        let rep = match a.style.as_str() {
            "label" => format!("[{}]", it["label"].as_str().unwrap_or("PII")),
            "keep" => orig.chars().take(a.keep).collect::<String>() + &"*".repeat(n.saturating_sub(a.keep)),
            _ => "*".repeat(n),
        };
        data.splice(s..e, rep.into_bytes());
    }
    match &a.out {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &data) { eprintln!("쓰지 못했습니다 {}: {}", p, e); return 2; }
            eprintln!("가린 본문을 {} 에 썼습니다 ({}곳)", p, items.len());
        }
        None => {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&data);
            if !data.ends_with(b"\n") { let _ = o.write_all(b"\n"); }
        }
    }
    0
}

//------------------------------------------------------------------
// 예문 검사 실행 — 라벨마다 그 라벨 하나만 켜서 기대값이 잡히는지
//
// -in: pii     = 불러온 라이브러리
// -in: verbose = true 면 통과한 줄도 찍는다
//
// -out: (usize, usize) = (통과 수, 전체 수)
// -out: error = 없음(검사 오류는 실패로 센다)
//------------------------------------------------------------------
fn run_demo(pii: &MoaiPii, verbose: bool) -> (usize, usize) {
    let mut ok = 0;
    for (label, sent, want) in DEMO {
        let (code, v) = raw(pii, sent.as_bytes(), &json!({"labels": [label], "values": true}));
        let got: Vec<String> = v["items"].as_array().cloned().unwrap_or_default().iter()
            .filter(|it| it["label"] == *label)
            .filter_map(|it| it["value"].as_str().map(String::from)).collect();
        let pass = code == 0 && got.len() == 1 && got[0] == *want;
        if pass { ok += 1; }
        if verbose || !pass {
            out!("  [{}] {:<18} {}  → {}", if pass { "PASS" } else { "FAIL" }, label, sent,
                     if got.is_empty() { "(없음)".to_string() } else { got.join(", ") });
        }
    }
    (ok, DEMO.len())
}

//------------------------------------------------------------------
// demo 명령
//
// -in: pii = 불러온 라이브러리
//
// -out: i32 = 0 전부 통과 · 1 실패 있음
// -out: error = 없음
//------------------------------------------------------------------
fn cmd_demo(pii: &MoaiPii) -> i32 {
    out!("라벨별 예문 검사 ({})", pii.version());
    let (ok, n) = run_demo(pii, true);
    out!("\n결과: {} / {} 통과", ok, n);
    if ok == n { 0 } else { 1 }
}

//------------------------------------------------------------------
// 시험 한 건 기록·출력
//
// -in: res  = (이름, 통과) 를 쌓는 목록
// -in: name = 시험 이름
// -in: pass = 통과 여부
//
// -out: 없음
// -out: error = 없음
//------------------------------------------------------------------
fn check(res: &mut Vec<(String, bool)>, name: &str, pass: bool) {
    out!("  [{}] {}", if pass { "PASS" } else { "FAIL" }, name);
    res.push((name.to_string(), pass));
}

//------------------------------------------------------------------
// selftest 명령 — 연동 전에 확인할 것들을 한 번에
//=> 1) 라벨별 예문 20개
//    2) 좌표 단위 3종이 이모지 섞인 본문에서 같은 값을 가리키는지
//    3) 옵션·오류: 건수만·값·모르는 라벨·별칭·깨진 UTF-8·본문 속 \0·빈 본문·틀린 옵션 JSON·today
//    4) 8 스레드 동시 호출 결과가 같은지
//
// -in: pii = 불러온 라이브러리
// -in: a   = 인자(쓰지 않음, 앞으로 옵션을 늘릴 자리)
//
// -out: i32 = 0 전부 통과 · 1 실패 있음
// -out: error = 없음
//------------------------------------------------------------------
fn cmd_selftest(pii: &MoaiPii, _a: &Args) -> i32 {
    out!("moaipii selftest — 라이브러리 {} · 시험 도구 {}\n  {}\n", pii.version(), VERSION, pii.path().display());
    let mut res: Vec<(String, bool)> = Vec::new();
    // 결과 JSON 의 counts 가 기대와 같은지
    let counts_eq = |v: &Value, want: Value| v["counts"] == want;

    out!("[1] 라벨별 예문");
    let (ok, n) = run_demo(pii, false);
    check(&mut res, &format!("라벨별 예문 {} / {}", ok, n), ok == n);

    out!("\n[2] 좌표 단위");
    // 이모지는 char 1 · utf16 2 · byte 4 칸이라 단위마다 좌표가 달라야 한다
    let emo = "홍길동 😀 010-1234-5678";
    for (unit, s, e) in [("char", 6u64, 19u64), ("utf16", 7, 20), ("byte", 15, 28)] {
        let (code, v) = raw(pii, emo.as_bytes(), &json!({"labels": ["PHONE"], "offsets": unit}));
        let it = &v["items"][0];
        let mut pass = code == 0 && it["start"].as_u64() == Some(s) && it["end"].as_u64() == Some(e);
        // byte 좌표는 실제로 잘라서 값이 맞는지까지 본다
        if unit == "byte" && pass { pass = &emo.as_bytes()[s as usize..e as usize] == b"010-1234-5678"; }
        check(&mut res, &format!("offsets={:<5} → ({}, {})", unit, s, e), pass);
    }

    out!("\n[3] 옵션·오류");
    let multi = "연락처 010-1234-5678, 메일 hong@example.co.kr, 또 010-9876-5432".as_bytes();
    let (c, v) = raw(pii, multi, &json!({"labels": ["PHONE", "EMAIL"], "items": false}));
    check(&mut res, "items=false 면 건수만", c == 0 && counts_eq(&v, json!({"PHONE": 2, "EMAIL": 1}))
          && v["items"].as_array().map_or(true, |x| x.is_empty()));
    let (c, v) = raw(pii, multi, &json!({"labels": ["EMAIL"], "values": true}));
    check(&mut res, "values=true 면 원문 값", c == 0 && v["items"][0]["value"] == "hong@example.co.kr");
    let (c, v) = raw(pii, multi, &json!({"labels": ["FAX"]}));
    check(&mut res, "모르는 라벨 → arg 오류(코드 1)", c == 1 && v["error"]["kind"] == "arg");
    let (c, v) = raw(pii, multi, &json!({"labels": ["PHONE", "FAX"], "unknown_labels": "ignore"}));
    check(&mut res, "모르는 라벨 ignore → unknown 에 알림", c == 0 && v["unknown"] == json!(["FAX"]));
    let (c, v) = raw(pii, "처방전 교부번호 202401150001 입니다.".as_bytes(), &json!({"labels": ["PRESCRIPTION"]}));
    check(&mut res, "별칭 PRESCRIPTION → 건수만(위치 없음)", c == 0 && counts_eq(&v, json!({"PRESCRIPTION": 1}))
          && v["items"].as_array().map_or(false, |x| x.is_empty()));
    let mut bad = "연락처 010-1234-5678 ".as_bytes().to_vec();
    bad.extend_from_slice(b"\xff\xfe ");
    bad.extend_from_slice("끝".as_bytes());
    let (c, v) = raw(pii, &bad, &json!({"labels": ["PHONE"]}));
    check(&mut res, "깨진 UTF-8 → utf8 오류(코드 2)", c == 2 && v["error"]["kind"] == "utf8");
    let (c, v) = raw(pii, &bad, &json!({"labels": ["PHONE"], "invalid_utf8": "replace"}));
    check(&mut res, "깨진 UTF-8 replace → 계속 검사", c == 0 && counts_eq(&v, json!({"PHONE": 1})));
    let (c, v) = raw(pii, "앞\0연락처 010-1234-5678".as_bytes(), &json!({"labels": ["PHONE"]}));
    check(&mut res, "본문 속 \\0 에서 안 잘림", c == 0 && counts_eq(&v, json!({"PHONE": 1})));
    let (c, v) = raw(pii, b"", &json!({"labels": ["PHONE"]}));
    check(&mut res, "빈 본문", c == 0 && counts_eq(&v, json!({})));
    let (c, v) = pii.scan_json(multi, "{ 옵션이 JSON 이 아님");
    check(&mut res, "틀린 옵션 JSON → arg 오류", c == 1 && v["error"]["kind"] == "arg");
    // 미래 생년월일 판단은 today 기준 — 2025-12-31 생은 2025-06-01 기준 미래, 2026-01-01 기준 과거
    let rrn = "주민 251231-3123456".as_bytes();
    let (c1, v1) = raw(pii, rrn, &json!({"labels": ["RRN"], "today": "2025-06-01"}));
    let (c2, v2) = raw(pii, rrn, &json!({"labels": ["RRN"], "today": "2026-01-01"}));
    check(&mut res, "today 기준 미래 생년월일 거부", c1 == 0 && c2 == 0
          && counts_eq(&v1, json!({})) && counts_eq(&v2, json!({"RRN": 1})));

    out!("\n[4] 스레드 동시 호출");
    let body: Vec<u8> = DEMO.iter().map(|d| d.1).collect::<Vec<_>>().join("\n").repeat(50).into_bytes();
    let all = pii.labels().unwrap_or_default();
    let o = json!({"labels": all, "items": false});
    let (_, base) = raw(pii, &body, &o);
    let same = AtomicBool::new(true);
    // 같은 라이브러리 한 벌을 8 스레드가 함께 써서 결과가 기준과 같은지
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| for _ in 0..5 {
                if raw(pii, &body, &o).1["counts"] != base["counts"] { same.store(false, Ordering::Relaxed); }
            });
        }
    });
    check(&mut res, "8 스레드 × 5회 결과 일치", same.load(Ordering::Relaxed) && base["counts"].is_object());

    let fails = res.iter().filter(|r| !r.1).count();
    out!("\n결과: {} / {} 통과", res.len() - fails, res.len());
    if fails == 0 { 0 } else { 1 }
}

//------------------------------------------------------------------
// bench 명령 — 처리 속도 측정
//=> 같은 본문을 repeat 번 검사한다. threads 가 2 이상이면 나눠 동시에 돌린다.
//   첫 호출은 준비 비용이 섞이므로 재기 전에 한 번 돌려 둔다. 위치 목록은 생략(건수만).
//
// -in: pii = 불러온 라이브러리
// -in: a   = 인자(repeat, threads, labels)
//
// -out: i32 = 0 성공 · 2 입력 오류 · 4 검사 오류
// -out: error = 위 종료 코드로 알림
//------------------------------------------------------------------
fn cmd_bench(pii: &MoaiPii, a: &Args) -> i32 {
    let data = match read_input(a) { Ok(d) => d, Err(e) => { eprintln!("{}", e); return 2; } };
    let labels = match parse_labels(pii, &a.labels) { Ok(l) => l, Err(e) => { eprintln!("[moaipii 오류] {}", e); return 4; } };
    let o = json!({"labels": labels, "items": false, "invalid_utf8": a.invalid_utf8});
    let (code, first) = raw(pii, &data, &o);
    if code != 0 { return lib_error(code, &first); }
    let per = (a.repeat / a.threads).max(1);
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..a.threads {
            s.spawn(|| for _ in 0..per { raw(pii, &data, &o); });
        }
    });
    let sec = t0.elapsed().as_secs_f64();
    let n = per * a.threads;
    let mb = data.len() as f64 * n as f64 / 1_048_576.0;
    out!("본문 {:.1} KB · 라벨 {}개 · {}회 × {}스레드 = {}회", data.len() as f64 / 1024.0, labels.len(), per, a.threads, n);
    out!("총 {:.3} s · 1회 평균 {:.2} ms(스레드당) · 처리량 {:.2} MB/s",
             sec, sec * 1000.0 / per as f64, mb / sec.max(1e-9));
    out!("건수: {}", first["counts"]);
    0
}

//------------------------------------------------------------------
// 진입점
//=> 인자를 해석해 명령을 고른다. 라이브러리가 필요 없는 help·version 은 불러오지 않는다.
//
// -in: 없음(명령줄 인자)
//
// -out: 없음(종료 코드로 끝낸다)
// -out: error = 인자 오류 2 · 불러오기 실패 3 (USAGE 의 종료 코드 표 참고)
//------------------------------------------------------------------
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => { eprintln!("{}\n\n{}", e, USAGE); std::process::exit(2); }
    };
    match a.cmd.as_str() {
        "help" => { let _ = write!(std::io::stdout(), "{}", USAGE); return; }
        "version" => { out!("moaipii-test {}", VERSION); return; }
        _ => {}
    }
    let known = ["info", "labels", "demo", "selftest", "scan", "mask", "bench"];
    if !known.contains(&a.cmd.as_str()) {
        eprintln!("모르는 명령: {}\n\n{}", a.cmd, USAGE);
        std::process::exit(2);
    }
    let pii = match load(&a) {
        Ok(p) => p,
        Err(e) => { eprintln!("[불러오기 실패] {}", e); std::process::exit(3); }
    };
    let code = match a.cmd.as_str() {
        "info" => cmd_info(&pii),
        "labels" => cmd_labels(&pii, &a),
        "demo" => cmd_demo(&pii),
        "selftest" => cmd_selftest(&pii, &a),
        "scan" => cmd_scan(&pii, &a),
        "mask" => cmd_mask(&pii, &a),
        _ => cmd_bench(&pii, &a),
    };
    // 표준출력을 비우고 끝낸다(process::exit 는 버퍼를 비우지 않을 수 있다)
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}
