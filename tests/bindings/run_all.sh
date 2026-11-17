#!/usr/bin/env bash
#------------------------------------------------------------------
# 감싸개 공통 시험 — 다섯 감싸개가 cases.json 의 정답과 같은 답을 내는가 (설계서 §5-5)
#=> 1) cases.json 이 make_cases.py 와 같은지 확인
#   2) 시험용 가짜 라이브러리(ABI 999) 빌드 → MOAIPII_FAKE_LIB
#   3) 언어별 실행기를 돌려 out/<언어>.jsonl 에 결과를 모은다
#        py       Python 감싸개                      (python 필요)
#        rs       Rust 감싸개                        (cargo 필요)
#        java     Java 감싸개(JNA)                   (javac·java·JNA jar 필요 — JNA_JAR 또는 third_party/java)
#        php      PHP 감싸개(FFI)                    (php + FFI 확장 필요)
#        cpp      C++ 감싸개 동적 로드형             (Windows: VS 2022 cl / Linux: g++ C++17)
#        cpp-link C++ 감싸개 링크형                  (위와 같음)
#      도구가 없는 언어는 "건너뜀(언어: 이유)" 를 찍고 넘어간다 — 조용히 넘어가면 시험 안 한 감싸개가 나간다.
#   4) compare.py 로 정답과 비교 — 하나라도 다르면 종료 코드 1
#
#   사용: MOAIPII_LIB=<moaipii.dll|libmoaipii.so> bash tests/bindings/run_all.sh
#   환경: JNA_JAR(선택) · VCVARS64(선택, Windows cl 환경 스크립트)
#------------------------------------------------------------------
set -uo pipefail
cd "$(dirname "$0")"
HERE="$PWD"
ROOT="$(cd ../.. && pwd)"

# 실패하면 사유를 남기고 멈춘다
die() { echo "[실패] $*" >&2; exit 1; }
# Windows(Git Bash)면 C:/… 꼴 경로로 바꾼다 — 네이티브 프로그램(java·cl·php)이 /d/… 를 못 읽는다
winpath() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi; }

: "${MOAIPII_LIB:?MOAIPII_LIB 에 시험할 라이브러리 경로를 주세요}"
[ -f "$MOAIPII_LIB" ] || die "라이브러리가 없습니다: $MOAIPII_LIB"
LIB="$(cd "$(dirname "$MOAIPII_LIB")" && pwd)/$(basename "$MOAIPII_LIB")"
export MOAIPII_LIB="$(winpath "$LIB")"
LIBDIR="$(dirname "$LIB")"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) WIN=1; SEP=';' ;; *) WIN=0; SEP=':' ;; esac
# Windows 의 python3 는 스토어 안내 껍데기일 수 있어, 실제로 도는지 보고 고른다
PY=""
for c in python3 python; do if command -v "$c" >/dev/null && "$c" -c "import sys" >/dev/null 2>&1; then PY="$c"; break; fi; done
[ -n "$PY" ] || die "python 이 필요합니다(사례 확인·비교)"

"$PY" -I make_cases.py --check >/dev/null || die "cases.json 이 make_cases.py 와 다릅니다 — python tests/bindings/make_cases.py"
OUT="$HERE/out"
rm -rf "$OUT"/*.jsonl "$OUT"/java "$OUT"/cpp* 2>/dev/null
mkdir -p "$OUT"
SKIPPED=()
RAN=()

echo "  가짜 라이브러리(ABI 999) 빌드"
cargo build -q --release --manifest-path fake_abi/Cargo.toml --target-dir "$OUT/fake-target" || die "가짜 라이브러리 빌드 실패"
if [ $WIN = 1 ]; then FAKE="$OUT/fake-target/release/moaipii_fake.dll"; else FAKE="$OUT/fake-target/release/libmoaipii_fake.so"; fi
[ -f "$FAKE" ] || die "가짜 라이브러리가 없습니다: $FAKE"
export MOAIPII_FAKE_LIB="$(winpath "$FAKE")"

# ---- py ----
"$PY" -I runner.py cases.json "$(winpath "$ROOT/bindings/python")" > "$OUT/py.jsonl" || die "Python 실행기 실패"
RAN+=(py)

# ---- rs ----
if command -v cargo >/dev/null; then
    cargo build -q --release --manifest-path rust/Cargo.toml --target-dir "$OUT/rs-target" || die "Rust 실행기 빌드 실패"
    "$OUT/rs-target/release/runner" cases.json > "$OUT/rs.jsonl" || die "Rust 실행기 실패"
    RAN+=(rs)
else
    SKIPPED+=("rs: cargo 없음")
fi

# ---- java ----
JNA="${JNA_JAR:-$ROOT/third_party/java/jna-5.14.0.jar}"
if ! command -v javac >/dev/null || ! command -v java >/dev/null; then
    SKIPPED+=("java: javac/java 없음")
elif [ ! -f "$JNA" ]; then
    SKIPPED+=("java: JNA jar 없음($JNA — JNA_JAR 로 알려 주세요)")
else
    mkdir -p "$OUT/java"
    javac -encoding UTF-8 -nowarn -d "$(winpath "$OUT/java")" -cp "$(winpath "$JNA")" \
        "$(winpath "$ROOT/bindings/java/moaipii/MoaiPii.java")" "$(winpath "$ROOT/bindings/java/moaipii/Json.java")" \
        "$(winpath "$HERE/java/Runner.java")" || die "Java 컴파일 실패"
    java -Dfile.encoding=UTF-8 -cp "$(winpath "$JNA")$SEP$(winpath "$OUT/java")" Runner cases.json > "$OUT/java.jsonl" || die "Java 실행기 실패"
    RAN+=(java)
fi

# ---- php ----
if ! command -v php >/dev/null; then
    SKIPPED+=("php: php 없음")
elif ! php -d ffi.enable=1 -r 'exit(extension_loaded("ffi") ? 0 : 1);' 2>/dev/null; then
    SKIPPED+=("php: FFI 확장 없음(php.ini 에 extension=ffi)")
else
    php -d ffi.enable=1 php/runner.php cases.json "$(winpath "$ROOT/bindings/php")" > "$OUT/php.jsonl" || die "PHP 실행기 실패"
    RAN+=(php)
fi

# ---- cpp · cpp-link ----
INC="$ROOT/include"
if [ $WIN = 1 ]; then
    VC="${VCVARS64:-}"
    if [ -z "$VC" ]; then
        for c in "/c/Program Files/Microsoft Visual Studio/2022/"*/VC/Auxiliary/Build/vcvars64.bat; do [ -f "$c" ] && VC="$c" && break; done
    fi
    if [ -z "$VC" ] || [ ! -f "$VC" ]; then
        SKIPPED+=("cpp: VS 2022 cl 없음(VCVARS64 로 vcvars64.bat 을 알려 주세요)")
    elif [ ! -f "$LIB.lib" ]; then
        SKIPPED+=("cpp: 가져오기 라이브러리 없음($LIB.lib)")
    else
        mkdir -p "$OUT/cpp"
        cp "$LIB" "$OUT/cpp/"   # 링크형은 실행할 때 exe 옆의 DLL 을 쓴다
        cat > "$OUT/cpp/build.bat" <<BAT
@echo off
call "$(cygpath -w "$VC")" >nul 2>&1 || exit /b 1
cd /d "$(cygpath -w "$OUT/cpp")"
cl /nologo /std:c++17 /EHsc /utf-8 /O2 /W3 /DMOAIPII_DYNAMIC /I "$(cygpath -w "$INC")" /I "$(cygpath -w "$ROOT/bindings/cpp")" "$(cygpath -w "$HERE/cpp/runner.cpp")" /Fe:cpp.exe >build-dyn.log || (type build-dyn.log & exit /b 1)
cl /nologo /std:c++17 /EHsc /utf-8 /O2 /W3 /I "$(cygpath -w "$INC")" /I "$(cygpath -w "$ROOT/bindings/cpp")" "$(cygpath -w "$HERE/cpp/runner.cpp")" /Fe:cpp-link.exe "$(cygpath -w "$LIB.lib")" >build-link.log || (type build-link.log & exit /b 1)
BAT
        cmd //c "$(cygpath -w "$OUT/cpp/build.bat")" || die "C++ 컴파일 실패"
        "$OUT/cpp/cpp.exe" cases.json > "$OUT/cpp.jsonl" || die "C++(동적 로드형) 실행기 실패"
        "$OUT/cpp/cpp-link.exe" cases.json > "$OUT/cpp-link.jsonl" || die "C++(링크형) 실행기 실패"
        RAN+=(cpp cpp-link)
    fi
else
    CXX="${CXX:-g++}"
    if ! command -v "$CXX" >/dev/null; then
        SKIPPED+=("cpp: $CXX 없음")
    elif ! echo 'int main(){ if constexpr (true) return 0; }' | "$CXX" -std=c++17 -x c++ - -o /dev/null 2>/dev/null; then
        SKIPPED+=("cpp: $CXX 가 C++17 을 못 함(CentOS 7 은 devtoolset 의 g++ 를 CXX 로)")
    else
        mkdir -p "$OUT/cpp"
        "$CXX" -std=c++17 -O2 -DMOAIPII_DYNAMIC -I "$INC" -I "$ROOT/bindings/cpp" cpp/runner.cpp -o "$OUT/cpp/cpp" -ldl -pthread || die "C++ 컴파일 실패"
        "$CXX" -std=c++17 -O2 -I "$INC" -I "$ROOT/bindings/cpp" cpp/runner.cpp -o "$OUT/cpp/cpp-link" -L "$LIBDIR" -lmoaipii -Wl,-rpath,"$LIBDIR" -pthread || die "C++ 링크형 컴파일 실패"
        "$OUT/cpp/cpp" cases.json > "$OUT/cpp.jsonl" || die "C++(동적 로드형) 실행기 실패"
        "$OUT/cpp/cpp-link" cases.json > "$OUT/cpp-link.jsonl" || die "C++(링크형) 실행기 실패"
        RAN+=(cpp cpp-link)
    fi
fi

echo "  비교 (돌린 언어: ${RAN[*]})"
files=()
for l in "${RAN[@]}"; do files+=("$OUT/$l.jsonl"); done
"$PY" -I compare.py cases.json "${files[@]}"
rc=$?
for s in "${SKIPPED[@]+"${SKIPPED[@]}"}"; do echo "  건너뜀($s)"; done
exit $rc
