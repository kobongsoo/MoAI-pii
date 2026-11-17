//! moaipii — 한국어 개인정보(PII) 검출 라이브러리 (MpowerClassify 에서 떼어 낸 독립 프로젝트)
//!
//! 설계: plan/MoAI-pii-개인정보검출-라이브러리분리-설계-20261117.html
//!
//! 검출 알고리즘(detect.rs)은 ko-pii 1.15.2 를 옮겨 적은 MpowerClassify `Rust/src/pii.rs` 를
//! 그대로 가져왔다(의도적 차이 A·A-2·B·G·H·I·J 포함). 이 파일은 그 위에
//!   · 좌표 단위 바꾸기(문자·UTF-16·바이트)
//!   · 건수 집계(처방전 별칭 포함)
//!   · 깨진 UTF-8 을 U+FFFD 로 보고 원래 바이트 좌표로 되돌리기
//! 를 얹어, C 진입점(moaipii-ffi)과 각 언어 감싸개가 같은 규칙을 쓰게 한다.

// 시험 함수 이름은 한글로 쓴다(MpowerClassify 와 같은 관례).
#![cfg_attr(test, allow(non_snake_case))]

mod detect;
mod districts;
mod fold;

use std::collections::{BTreeMap, HashSet};

pub use detect::{normalize_with_map, pii_counts, pii_records, scan_bytes, with_today, KOPII_PORTED_FROM};

/// 이 라이브러리가 검출할 수 있는 라벨(ko-pii detect.DETECTORS 차례).
pub const LABELS: &[&str] = &[
    "RRN", "FRN", "BUSINESS_REG", "CORP_REG", "DRIVER_LICENSE", "PASSPORT", "CARD",
    "MEDICAL_INSURANCE", "PRESCRIPTION_ID", "PNU", "PHONE", "EMAIL", "IP", "VEHICLE",
    "URL", "ADDRESS", "NATIONALITY", "ACCOUNT", "EDI_DRUG", "COURT_CASE",
];

/// 별칭 → 실제 검출 라벨. 별칭을 켜면 건수만 별칭 이름으로도 함께 센다(pii_counts 규칙).
pub const ALIASES: &[(&str, &str)] = &[("PRESCRIPTION", "PRESCRIPTION_ID")];

//------------------------------------------------------------------
// 아는 라벨인가
//=> 검출 라벨이거나 별칭이면 true. 모르는 라벨을 조용히 무시하면 "켰는데 안 잡힌다"를
//   알 수 없으므로, C 진입점은 기본으로 이것으로 걸러 오류를 낸다.
//
// -in: label = 라벨 이름
//
// -out: bool = 아는 라벨이면 true
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn is_known_label(label: &str) -> bool {
    LABELS.contains(&label) || ALIASES.iter().any(|(a, _)| *a == label)
}

/// 좌표 단위 — 부르는 언어가 자기 문자열을 세는 단위로 고른다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offsets {
    /// 유니코드 코드포인트 (Python str · Rust chars)
    Char,
    /// UTF-16 코드 단위 (Java String · JavaScript)
    Utf16,
    /// 넘긴 UTF-8 바이트 (PHP · C++ std::string)
    Byte,
}

impl Offsets {
    //------------------------------------------------------------------
    // 이름 → 좌표 단위
    //=> 옵션 JSON 의 "offsets" 값을 읽는다.
    //
    // -in: s = "char" | "utf16" | "byte"
    //
    // -out: Some(단위) / 모르는 이름이면 None
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn parse(s: &str) -> Option<Offsets> {
        match s {
            "char" => Some(Offsets::Char),
            "utf16" => Some(Offsets::Utf16),
            "byte" => Some(Offsets::Byte),
            _ => None,
        }
    }
}

/// 검사 옵션.
#[derive(Clone, Debug)]
pub struct Opts {
    /// false 면 위치 목록을 만들지 않는다(건수만 — 큰 문서에서 결과를 줄인다)
    pub items: bool,
    /// true 면 위치마다 원문 값을 싣는다(--with-pii 전용)
    pub values: bool,
    /// 좌표 단위
    pub offsets: Offsets,
    /// 미래 생년월일 판단 기준일. None = 시스템 날짜
    pub today: Option<(i64, u32, u32)>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { items: true, values: false, offsets: Offsets::Char, today: None }
    }
}

/// 검출 한 건.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub start: usize,
    pub end: usize,
    pub value: Option<String>,
}

/// 검사 결과.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// 본문 길이(코드포인트)
    pub chars: usize,
    /// 라벨별 건수(0 건은 담지 않는다)
    pub counts: BTreeMap<String, u32>,
    /// 겹침 해소가 끝난 위치 목록(문서 순서). opts.items=false 면 비어 있다
    pub items: Vec<Item>,
}

//------------------------------------------------------------------
// 본문 검사 — 건수와 위치를 한 번의 스캔에서
//=> 1) detect::scan_bytes 로 겹침 해소가 끝난 (라벨, 바이트 좌표) 목록을 받는다
//    2) 건수는 pii_counts 와 같은 규칙으로 센다(wanted 에 있는 라벨 + 처방전 별칭)
//    3) 위치는 pii_records 와 같은 규칙으로 고른다(wanted 에 있는 라벨만)
//    4) 좌표를 opts.offsets 단위로 바꾼다(본문을 한 번만 훑는다)
//   [왜 한 스캔인가] 건수와 위치를 따로 구하면 서로 어긋날 수 있다
//   (2026-09-11 Python 판에서 "전화 7건인데 값은 9개"가 실제로 나왔다).
//
// -in: text   = 본문(올바른 UTF-8)
// -in: wanted = 켤 라벨 집합(별칭 포함 가능, 모르는 라벨은 그냥 안 잡힌다)
// -in: opts   = 옵션(Opts::default() = 위치 있음·값 없음·문자 좌표·시스템 날짜)
//
// -out: Scan = 길이·건수·위치
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn scan(text: &str, wanted: &HashSet<String>, opts: &Opts) -> Scan {
    let hits = with_today(opts.today, || scan_bytes(text, wanted));
    let mut out = Scan { chars: text.chars().count(), ..Default::default() };

    // 건수 — pii_counts 와 같은 규칙(별칭 PRESCRIPTION 은 PRESCRIPTION_ID 검출로 함께 센다)
    for &(label, _, _) in &hits {
        if wanted.contains(label) {
            *out.counts.entry(label.to_string()).or_insert(0) += 1;
        }
        for (alias, real) in ALIASES {
            if label == *real && wanted.contains(*alias) {
                *out.counts.entry(alias.to_string()).or_insert(0) += 1;
            }
        }
    }
    if !opts.items {
        return out;
    }

    // 위치 — pii_records 와 같은 규칙(wanted 에 있는 라벨만)
    let picked: Vec<(&'static str, usize, usize)> =
        hits.into_iter().filter(|(l, _, _)| wanted.contains(*l)).collect();
    let mut marks: Vec<usize> = Vec::with_capacity(picked.len() * 2);
    for &(_, s, e) in &picked { marks.push(s); marks.push(e); }
    let conv = convert_marks(text, marks, opts.offsets);
    out.items = picked
        .into_iter()
        .map(|(label, s, e)| Item {
            label,
            start: conv(s),
            end: conv(e),
            // 값은 원본 바이트 좌표로 떠낸다(단위 변환 전 좌표).
            value: if opts.values { Some(text[s..e].to_string()) } else { None },
        })
        .collect();
    out
}

//------------------------------------------------------------------
// 바이트 좌표 묶음 → 원하는 단위의 좌표로 바꾸는 함수 만들기
//=> 필요한 경계만 모아 본문을 한 번만 훑는다. 검출마다 앞에서부터 다시 세면
//   문서가 길수록 제곱으로 느려진다. ASCII 본문이면 모든 단위가 같아 훑지 않는다.
//   글자 중간을 가리키는 경계가 오면 그 글자 자리로 접는다(detect::to_char_offsets 와 같은 규칙).
//
// -in: text   = 좌표의 기준 본문
// -in: marks  = 바꿀 바이트 좌표들(순서·중복 상관없음)
// -in: unit   = 바꿀 단위
//
// -out: 바이트 좌표 → 변환 좌표 함수 (marks 에 없던 값은 그대로 돌려준다)
// -out: error = 예외 없음
//------------------------------------------------------------------
fn convert_marks(text: &str, mut marks: Vec<usize>, unit: Offsets) -> impl Fn(usize) -> usize {
    let mut table: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    if unit != Offsets::Byte && !text.is_ascii() && !marks.is_empty() {
        marks.sort_unstable();
        marks.dedup();
        let mut mi = 0usize;
        let mut pos = 0usize; // 지금 글자의 변환 좌표
        for (bi, c) in text.char_indices() {
            // '<=' — 글자 중간을 가리키는 경계도 이 글자 자리로 접는다.
            while mi < marks.len() && marks[mi] <= bi {
                table.insert(marks[mi], pos);
                mi += 1;
            }
            if mi >= marks.len() { break; }
            pos += match unit { Offsets::Utf16 => c.len_utf16(), _ => 1 };
        }
        // 본문 끝을 가리키는 경계(마지막 검출의 end)는 위 루프가 못 만난다.
        while mi < marks.len() {
            table.insert(marks[mi], pos);
            mi += 1;
        }
    }
    move |b| *table.get(&b).unwrap_or(&b)
}

/// 깨진 UTF-8 을 고친 본문과, 고친 본문 바이트 좌표 → 원래 바이트 좌표 표.
pub struct Lossy {
    pub text: String,
    /// (고친 본문의 시작 바이트, 원래 입력의 시작 바이트, 이 조각이 U+FFFD 인가)
    segs: Vec<(usize, usize, bool)>,
    orig_len: usize,
}

impl Lossy {
    //------------------------------------------------------------------
    // 고친 본문의 바이트 좌표 → 원래 입력의 바이트 좌표
    //=> 올바른 조각은 같은 거리만큼 밀린 자리로, U+FFFD 조각 안쪽은 그 깨진 바이트의
    //   시작으로 돌린다. 본문 끝은 원래 입력 끝이다.
    //
    // -in: b = 고친 본문의 바이트 좌표
    //
    // -out: 원래 입력의 바이트 좌표
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn to_orig(&self, b: usize) -> usize {
        if b >= self.text.len() { return self.orig_len; }
        // b 를 담은 조각 찾기(조각은 시작 순으로 정렬돼 있다)
        let i = match self.segs.binary_search_by(|s| s.0.cmp(&b)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let (ns, os, bad) = self.segs[i];
        if bad { os } else { os + (b - ns) }
    }
}

//------------------------------------------------------------------
// 깨진 UTF-8 을 U+FFFD 로 고치기 (원래 좌표표와 함께)
//=> PHP 처럼 문자열이 UTF-8 이 아닐 수 있는 언어를 위해, 깨진 바이트 묶음 하나를
//   U+FFFD 한 글자로 보고 검사한다. byte 좌표를 요청한 쪽에는 원래 입력의 바이트를
//   돌려줘야 substr 가 맞으므로 좌표표를 함께 만든다.
//
// -in: bytes = 입력 바이트
//
// -out: Lossy = 고친 본문 + 좌표표
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn decode_lossy(bytes: &[u8]) -> Lossy {
    let mut text = String::with_capacity(bytes.len());
    let mut segs = Vec::new();
    let mut orig = 0usize;
    for chunk in bytes.utf8_chunks() {
        let v = chunk.valid();
        if !v.is_empty() {
            segs.push((text.len(), orig, false));
            text.push_str(v);
            orig += v.len();
        }
        let bad = chunk.invalid();
        if !bad.is_empty() {
            segs.push((text.len(), orig, true));
            text.push('\u{FFFD}');
            orig += bad.len();
        }
    }
    Lossy { text, segs, orig_len: bytes.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 편의 — 라벨 목록을 집합으로
    fn set(ls: &[&str]) -> HashSet<String> { ls.iter().map(|s| s.to_string()).collect() }

    // 설계서 §5-2 의 예: 이모지 앞뒤 전화번호 좌표가 단위마다 맞는가
    #[test]
    fn 좌표_단위별로_맞다() {
        let t = "홍길동 😀 010-1234-5678";
        let w = set(&["PHONE"]);
        let mut o = Opts::default();
        let c = scan(t, &w, &o);
        assert_eq!((c.items[0].start, c.items[0].end), (6, 19));
        o.offsets = Offsets::Utf16;
        let u = scan(t, &w, &o);
        assert_eq!((u.items[0].start, u.items[0].end), (7, 20));
        o.offsets = Offsets::Byte;
        o.values = true;
        let b = scan(t, &w, &o);
        assert_eq!((b.items[0].start, b.items[0].end), (15, 28));
        assert_eq!(&t[15..28], "010-1234-5678");
        assert_eq!(b.items[0].value.as_deref(), Some("010-1234-5678"));
        assert_eq!(c.counts.get("PHONE"), Some(&1));
    }

    // 문자 좌표·값이 예전 pii_records 와 똑같은가 (MpowerClassify Rust 판 결과 동일 조건)
    #[test]
    fn 문자좌표는_pii_records_와_같다() {
        let t = "담당: 홍길동 010-1234-5678, 사업자 022-00-76422 메일 a.b@example.co.kr";
        let w = set(&["PHONE", "BUSINESS_REG", "EMAIL"]);
        let o = Opts { values: true, ..Default::default() };
        let got: Vec<(&str, String, usize, usize)> = scan(t, &w, &o).items.into_iter()
            .map(|i| (i.label, i.value.unwrap(), i.start, i.end)).collect();
        assert_eq!(got, pii_records(t, &w));
        let counts: std::collections::HashMap<String, u32> = scan(t, &w, &Opts::default()).counts.into_iter().collect();
        assert_eq!(counts, pii_counts(t, &w));
    }

    // 별칭만 켜면 건수는 별칭으로 세고, 위치는 싣지 않는다(pii_records 와 같은 규칙)
    #[test]
    fn 처방전_별칭() {
        let t = "처방전 교부번호 2024-01234-56789";
        let w = set(&["PRESCRIPTION"]);
        let s = scan(t, &w, &Opts::default());
        assert_eq!(s.counts.get("PRESCRIPTION_ID"), None);
        assert_eq!(pii_counts(t, &w).get("PRESCRIPTION").copied(), s.counts.get("PRESCRIPTION").copied());
        assert!(s.items.is_empty());
    }

    // 깨진 바이트가 앞에 있어도 byte 좌표는 원래 입력을 가리킨다
    #[test]
    fn 깨진_utf8_좌표는_원래_바이트() {
        let mut b: Vec<u8> = vec![0xC0, 0xFF, b' '];
        b.extend_from_slice("전화 010-1234-5678".as_bytes());
        let l = decode_lossy(&b);
        assert!(l.text.starts_with('\u{FFFD}'));
        let o = Opts { offsets: Offsets::Byte, ..Default::default() };
        let s = scan(&l.text, &set(&["PHONE"]), &o);
        let (st, en) = (l.to_orig(s.items[0].start), l.to_orig(s.items[0].end));
        assert_eq!(&b[st..en], b"010-1234-5678");
    }

    // 기준일 고정: 미래 생년월일 판단이 '오늘'에 따라 바뀐다
    #[test]
    fn 기준일_옵션() {
        // 2030-01-01 생(내국인 남 3) — 체크섬이 맞는 번호를 찾아 쓴다
        let front = "300101";
        let mut found = None;
        for tail in 0..1_000_000u32 {
            let s = format!("{}-3{:06}", front, tail);
            let d: Vec<u32> = s.chars().filter(|c| c.is_ascii_digit()).map(|c| c.to_digit(10).unwrap()).collect();
            let w = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5];
            let sum: u32 = (0..12).map(|i| d[i] * w[i]).sum();
            if (11 - sum % 11) % 10 == d[12] { found = Some(s); break; }
        }
        let rrn = found.unwrap();
        let t = format!("주민등록번호 {}", rrn);
        let w = set(&["RRN"]);
        let past = Opts { today: Some((2026, 11, 17)), ..Default::default() };
        let future = Opts { today: Some((2031, 1, 1)), ..Default::default() };
        let a = scan(&t, &w, &past).counts.get("RRN").copied().unwrap_or(0);
        let b = scan(&t, &w, &future).counts.get("RRN").copied().unwrap_or(0);
        // 2026 기준으로는 아직 태어나지 않은 날이라 버리고, 2031 기준으로는 잡는다.
        assert_eq!((a, b), (0, 1), "기준일에 따라 판단이 바뀌어야 한다");
    }
}
