#!/usr/bin/env bash
#------------------------------------------------------------------
# Windows 빌드 — 이 PC 에서 moaipii.dll 을 만들고 배포 묶음(zip)으로 낸다
#=> 1) cargo test --workspace (본체·골든·C 진입점 시험)
#   2) cargo build --release -p moaipii-ffi -p moaipii-cli -p moaipii-test
#      → target/release/moaipii.dll · mp.exe · moaipii-test.exe
#   3) Python 감싸개로 불러 보기(ABI·버전·한 건 검사), mp.exe 로 같은 버전·검사 확인,
#      moaipii-test.exe selftest 로 빌드한 DLL 을 실제로 불러 전 항목 시험
#   4) 감싸개 공통 시험(tests/bindings/run_all.sh) — 도구가 없는 언어는 이름을 찍고 건너뜀
#   5) scripts/package.py 로 dist/moaipii-<버전>-windows-x64.zip (mp.exe · moaipii-test.exe 포함)
#
#   사용: bash scripts/win-build.sh   (Git Bash)
#------------------------------------------------------------------
set -euo pipefail
cd "$(dirname "$0")/.."

# 실패하면 사유를 남기고 멈춘다
die() { echo "[실패] $*" >&2; exit 1; }
# 단계 머리줄
step() { echo ""; echo "── $* ──"; }

command -v cargo >/dev/null || die "cargo 를 찾을 수 없습니다"

# 버전 = 크레이트 버전 + git 해시(커밋 안 된 수정이 있으면 -dirty). build.rs 와 같은 규칙이다.
VER_CRATE="$(grep -m1 '^version' crates/moaipii-ffi/Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
HASH="$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then HASH="$HASH-dirty"; fi
VERSION="$VER_CRATE+$HASH"
export MOAIPII_GIT="$HASH"

step "시험"
cargo test --workspace --release >/dev/null 2>&1 || die "cargo test 실패 — 'cargo test --workspace --release' 로 보세요"

step "릴리스 빌드 ($VERSION)"
cargo build --release -p moaipii-ffi -p moaipii-cli -p moaipii-test >/dev/null 2>&1 || die "cargo build 실패"
LIB=target/release/moaipii.dll
CLI=target/release/mp.exe
TST=target/release/moaipii-test.exe
[ -f "$LIB" ] || die "빌드 결과가 없습니다: $LIB"
[ -f "$CLI" ] || die "빌드 결과가 없습니다: $CLI"
[ -f "$TST" ] || die "빌드 결과가 없습니다: $TST"

step "불러 보기"
MOAIPII_LIB="$PWD/$LIB" PYTHONIOENCODING=utf-8 python -I - "$PWD/bindings/python" "$VERSION" <<'PY' \
    || die "Python 감싸개로 불러 보기 실패"
import sys
sys.path.insert(0, sys.argv[1])
import moaipii
assert moaipii.version() == sys.argv[2], (moaipii.version(), sys.argv[2])
r = moaipii.scan("연락처 010-1234-5678", ["PHONE"])
assert r["counts"] == {"PHONE": 1}, r
print("  ", moaipii.version(), r["counts"])
PY
# mp.exe 도 DLL 과 같은 버전 · 같은 검사 결과를 내는지 본다
tmp="$(mktemp -d)"
printf '연락처 010-1234-5678\n' > "$tmp/a.txt"
"$CLI" version | grep -qF "$VERSION" || die "mp.exe 버전이 $VERSION 이 아닙니다"
"$CLI" scan "$tmp/a.txt" --labels PHONE | grep -q '"PHONE":1' || die "mp.exe 로 검사 실패"
echo "   $("$CLI" version)"
rm -rf "$tmp"
# 배포 시험 도구가 옆의 DLL 을 실제로 불러 전 항목을 통과하는지 (묶음을 받은 쪽이 돌릴 것과 같은 시험)
"$TST" version | grep -qF "$VERSION" || die "moaipii-test.exe 버전이 $VERSION 이 아닙니다"
MOAIPII_LIB= "$TST" selftest > target/release/selftest.log 2>&1 \
    || { tail -20 target/release/selftest.log; die "moaipii-test.exe selftest 실패"; }
echo "   moaipii-test $(tail -1 target/release/selftest.log)"

step "감싸개 공통 시험"
MOAIPII_LIB="$PWD/$LIB" bash tests/bindings/run_all.sh || die "감싸개 공통 시험 실패"

step "묶기"
python scripts/package.py windows-x64 "$LIB" "$VERSION" "$CLI" "$TST"
