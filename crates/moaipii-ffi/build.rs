//! 빌드 스크립트 — 라이브러리 버전 문자열에 붙일 git 해시를 새긴다.
//!
//! moaipii_version() 이 "0.1.0+abc1234" 처럼 돌려줘서, 부른 프로그램이 로그·레코드에
//! "어느 검출기로 검사했나" 를 남길 수 있게 한다. 커밋 안 된 수정이 있으면 "-dirty" 를 붙인다.

//------------------------------------------------------------------
// 빌드 진입점
//=> git 해시를 환경변수 MOAIPII_GIT 로 컴파일러에 넘긴다.
//    1) 환경변수 MOAIPII_GIT 이 있으면 그 값, 없으면 git rev-parse --short HEAD (git 이 없으면 "nogit")
//    2) 추적 중인 파일에 수정이 있으면 "-dirty"
//
// -in: 없음
//
// -out: 없음(cargo 지시문을 표준출력으로 낸다)
// -out: error = 없음(git 실패는 "nogit" 으로 대신)
//------------------------------------------------------------------
fn main() {
    // git 명령 한 번 실행해 성공하면 출력 한 줄을 돌려주는 작은 도우미
    let run = |args: &[&str]| {
        std::process::Command::new("git").args(args).output().ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // 빌드서버처럼 .git 없이 소스만 올려 빌드할 때는 올린 쪽이 해시를 환경변수로 넘긴다.
    println!("cargo:rerun-if-env-changed=MOAIPII_GIT");
    let hash = match std::env::var("MOAIPII_GIT") {
        Ok(h) if !h.trim().is_empty() => h.trim().to_string(),
        _ => {
            let mut h = run(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "nogit".to_string());
            // 수정된 채로 빌드한 것은 구분해 둔다 — 같은 해시인데 결과가 다를 수 있기 때문이다.
            if run(&["status", "--porcelain", "--untracked-files=no"]).map_or(false, |s| !s.is_empty()) {
                h.push_str("-dirty");
            }
            h
        }
    };
    println!("cargo:rustc-env=MOAIPII_GIT={}", hash);
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    // 파일 안에 남길 버전 표식 — strings · readelf 로 실행하지 않고 판을 알 수 있게 한다.
    // ASCII 만 쓴다(strings 기본 설정은 ASCII 만 보여 준다).
    let win = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let file = match std::env::var("CARGO_PKG_NAME").unwrap_or_default().as_str() {
        "moaipii-ffi" => if win { "moaipii.dll" } else { "libmoaipii.so" },
        "moaipii-test" => if win { "moaipii-test.exe" } else { "moaipii-test" },
        "moaipii-cli" => if win { "mp.exe" } else { "mp" },
        other => other,
    }.to_string();
    println!("cargo:rustc-env=MOAIPII_MARK=MoAI-pii {} {}+{} Copyright (C) 2026 kobongsoo",
             file, std::env::var("CARGO_PKG_VERSION").unwrap_or_default(), hash);

    // Windows(MSVC) 대상이면 탐색기 '속성 > 자세히' 에 보일 버전 정보를 실행파일·DLL 에 넣는다
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        win_version_resource(&hash);
    }
}

//------------------------------------------------------------------
// Windows 버전 정보 리소스(VERSIONINFO) 넣기
//=> Rust 빌드는 버전 리소스를 넣지 않아 탐색기 속성의 파일 버전·제품 이름·제품 버전·저작권·언어가
//   비어 보인다. 크레이트마다 맞는 설명을 골라 .rc 파일을 만들고, Windows SDK 의 rc.exe 로 컴파일해
//   링크에 붙인다(DLL(cdylib)에도 붙도록 compile_for_everything).
//    1) 크레이트 이름으로 파일 이름·설명·종류(DLL/EXE)를 고른다
//    2) 파일 버전 = Cargo 버전 숫자 4자리(0.1.0.0), 제품 버전 문자열 = "0.1.0+해시"
//    3) 언어 = 한국어(0x0412), 문자열은 유니코드(1200)
//
// -in: hash = git 해시(-dirty 포함)
//
// -out: 없음(OUT_DIR 에 version.rc 를 쓰고 cargo 링크 지시문을 낸다)
// -out: error = rc.exe 를 못 찾거나 컴파일 실패면 embed-resource 가 빌드를 멈춘다(panic)
//------------------------------------------------------------------
#[cfg(windows)]
fn win_version_resource(hash: &str) {
    let pkg = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // (파일 이름, 설명, 파일 종류 1=EXE 2=DLL)
    let (file, desc, ftype) = match pkg.as_str() {
        "moaipii-ffi" => ("moaipii.dll", "moaipii — 한국어 개인정보(PII) 검출 라이브러리", 2),
        "moaipii-test" => ("moaipii-test.exe", "moaipii-test — moaipii 라이브러리 시험 도구", 1),
        "moaipii-cli" => ("mp.exe", "mp — moaipii 개발용 시험 도구", 1),
        _ => return,
    };
    // "0.1.0" → 0,1,0,0 (숫자가 아닌 꼬리 -beta 등은 버린다)
    let mut n: Vec<u16> = ver.split(|c| c == '.' || c == '-' || c == '+')
        .take(3).map(|s| s.parse().unwrap_or(0)).collect();
    n.resize(4, 0);
    let fixed = format!("{},{},{},{}", n[0], n[1], n[2], n[3]);
    let dotted = format!("{}.{}.{}.{}", n[0], n[1], n[2], n[3]);
    // .rc 안 문자열의 따옴표는 두 번 써야 한다
    let q = |s: &str| s.replace('"', "\"\"");
    let rc = format!(r#"#pragma code_page(65001)
1 VERSIONINFO
FILEVERSION {fixed}
PRODUCTVERSION {fixed}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE {ftype}
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041204b0"
    BEGIN
      VALUE "FileDescription", "{desc}"
      VALUE "FileVersion", "{dotted}"
      VALUE "InternalName", "{internal}"
      VALUE "LegalCopyright", "Copyright © 2026 kobongsoo"
      VALUE "OriginalFilename", "{file}"
      VALUE "ProductName", "MoAI-pii"
      VALUE "ProductVersion", "{pver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x412, 1200
  END
END
"#, desc = q(desc), internal = q(file.trim_end_matches(".exe").trim_end_matches(".dll")),
        file = q(file), pver = q(&format!("{}+{}", ver, hash)));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("version.rc");
    std::fs::write(&out, rc.as_bytes()).expect("version.rc 를 쓰지 못했습니다");
    embed_resource::compile_for_everything(&out, embed_resource::NONE);
}

// Windows 가 아닌 곳에서 빌드할 때는 할 일이 없다(위 호출은 대상이 Windows 일 때만 일어난다)
#[cfg(not(windows))]
fn win_version_resource(_hash: &str) {}
