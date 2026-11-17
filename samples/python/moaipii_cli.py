# -*- coding: utf-8 -*-
"""moaipii_cli — moaipii.dll / libmoaipii.so 를 실제로 불러 시험하는 CLI 샘플

mp.exe 는 C 진입점 소스를 '안에 넣어' 만든 도구라 DLL 파일 자체를 부르지 않는다.
이 샘플은 배포된 DLL 파일을 Python 감싸개(bindings/python/moaipii.py)로 '밖에서' 불러
연동하는 쪽과 똑같은 길로 시험한다. 표준 라이브러리만 쓴다(Python 3.8+).

    python moaipii_cli.py info                         # 불러온 DLL 경로·버전·ABI
    python moaipii_cli.py labels                       # 지원 라벨
    python moaipii_cli.py demo                         # 라벨별 예문 20개로 검출 확인(PASS/FAIL)
    python moaipii_cli.py scan --text "연락처 010-1234-5678"
    python moaipii_cli.py scan 본문.txt --labels RRN,PHONE --offsets utf16
    python moaipii_cli.py mask 본문.txt --style label  > 가린본문.txt
    python moaipii_cli.py selftest                     # demo + 좌표·오류·스레드 시험
    python moaipii_cli.py bench 큰본문.txt --repeat 50 --threads 4

DLL 위치: --lib <파일|폴더> → 환경변수 MOAIPII_LIB → ../../lib(배포 묶음) → ../../target/release(저장소)
          → 감싸개 폴더 → 그 아래 lib/ → 실행 폴더.
"""
import argparse
import json
import os
import sys
import threading
import time


#------------------------------------------------------------------
# 감싸개(moaipii.py) 를 찾아 import
#=> 샘플은 저장소(samples/python) 와 배포 묶음 어디에 두어도 돌도록 감싸개를 몇 군데에서 찾는다.
#    1) 환경변수 MOAIPII_PY(감싸개 폴더)
#    2) 이 파일 폴더(감싸개를 옆에 복사해 둔 경우)
#    3) ../../bindings/python (저장소·배포 묶음 구조)
#
# -in: 없음
#
# -out: module = moaipii 감싸개 모듈
# -out: error = 어디에도 없으면 메시지를 찍고 종료코드 3 으로 끝낸다
#------------------------------------------------------------------
def import_binding():
    here = os.path.dirname(os.path.abspath(__file__))
    cands = [os.environ.get("MOAIPII_PY"), here,
             os.path.normpath(os.path.join(here, "..", "..", "bindings", "python")),
             os.path.normpath(os.path.join(here, "..", "bindings", "python"))]
    for d in cands:
        if d and os.path.isfile(os.path.join(d, "moaipii.py")):
            # 찾은 폴더를 맨 앞에 넣어 같은 이름의 다른 모듈보다 먼저 잡히게 한다
            sys.path.insert(0, d)
            import moaipii  # noqa: E402
            return moaipii
    sys.stderr.write("[오류] 감싸개 moaipii.py 를 찾지 못했습니다. 찾아본 곳: %s\n"
                     "       환경변수 MOAIPII_PY 로 폴더를 알려 주세요.\n" % ", ".join(d for d in cands if d))
    sys.exit(3)


mp = import_binding()


#------------------------------------------------------------------
# --lib 를 안 줬을 때 쓸 DLL 위치 고르기
#=> 감싸개 기본 탐색(MOAIPII_LIB → 감싸개 옆 → 감싸개/lib)에 더해, 이 샘플이 놓이는 자리에서
#   흔한 위치 두 곳을 더 본다. 그래야 묶음을 풀자마자 환경변수 없이 바로 돈다.
#    1) --lib 를 줬거나 MOAIPII_LIB 가 있으면 그대로 맡긴다
#    2) ../../lib       (배포 묶음: samples/python ↔ lib/)
#    3) ../../target/release (저장소에서 cargo build 한 직후)
#
# -in: given = --lib 값(없으면 None)
#
# -out: str|None = 쓸 경로(None 이면 감싸개 기본 탐색에 맡김)
# -out: error = 없음
#------------------------------------------------------------------
def default_lib(given):
    if given or os.environ.get("MOAIPII_LIB"):
        return given
    here = os.path.dirname(os.path.abspath(__file__))
    name = mp._lib_name()
    for d in (os.path.join(here, "..", "..", "lib"), os.path.join(here, "..", "..", "target", "release")):
        p = os.path.normpath(os.path.join(d, name))
        if os.path.isfile(p):
            return p
    return None

# 라벨별 예문 — 번호는 모두 체크섬만 맞춘 가짜값이다(실제 개인정보 아님).
# (라벨, 예문, 잡혀야 할 값)
DEMO = [
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
]


#------------------------------------------------------------------
# 표준출력·오류를 UTF-8 로
#=> Windows 콘솔(cp949)에서 한글·이모지를 찍다가 UnicodeEncodeError 가 나지 않게 한다.
#
# -in: 없음
#
# -out: 없음
# -out: error = 없음(재설정이 안 되는 옛 환경이면 그대로 둔다)
#------------------------------------------------------------------
def utf8_console():
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass


#------------------------------------------------------------------
# --labels 값 → 라벨 목록
#=> "all"(기본)이면 DLL 이 지원한다고 알려 준 라벨 전부, 아니면 쉼표로 나눈다.
#
# -in: spec = "all" 또는 "RRN,PHONE" 같은 문자열
#
# -out: list[str] = 켤 라벨들
# -out: error = DLL 을 못 불러오면 MoaiPiiError
#------------------------------------------------------------------
def parse_labels(spec):
    if not spec or spec.lower() == "all":
        return list(mp.labels()["labels"])
    return [x.strip().upper() for x in spec.split(",") if x.strip()]


#------------------------------------------------------------------
# 검사할 본문 읽기
#=> --text 가 있으면 그것, 아니면 파일(또는 "-" = 표준입력)을 읽는다.
#   파일은 bytes 로 읽은 뒤 --encoding 으로 str 로 바꾼다(EUC-KR 문서는 --encoding cp949).
#
# -in: args = argparse 결과(text, file, encoding)
#
# -out: str = 본문
# -out: error = 둘 다 없으면 SystemExit(2), 파일이 없으면 OSError 전파, 디코딩 실패면 UnicodeDecodeError
#------------------------------------------------------------------
def read_text(args):
    if args.text is not None:
        return args.text
    if not args.file:
        sys.stderr.write("[오류] 검사할 파일 또는 --text 를 주세요.\n")
        sys.exit(2)
    if args.file == "-":
        data = sys.stdin.buffer.read()
    else:
        with open(args.file, "rb") as f:
            data = f.read()
    # BOM 이 붙은 UTF-8 은 utf-8-sig 로 읽어야 첫 글자 좌표가 밀리지 않는다
    enc = "utf-8-sig" if args.encoding.lower() in ("utf-8", "utf8") else args.encoding
    return data.decode(enc)


#------------------------------------------------------------------
# 좌표 → 원문 조각
#=> DLL 이 준 좌표 단위(offsets)에 맞춰 Python str 에서 값을 잘라 낸다.
#   char 는 그대로, utf16·byte 는 그 단위로 바꾼 버퍼에서 잘라 다시 str 로 돌린다.
#
# -in: text    = 본문 str
# -in: start   = 시작 좌표
# -in: end     = 끝 좌표(포함 안 함)
# -in: offsets = "char" | "utf16" | "byte"
#
# -out: str = 잘라 낸 값
# -out: error = 없음(깨진 조각은 대체 문자로)
#------------------------------------------------------------------
def slice_by(text, start, end, offsets):
    if offsets == "char":
        return text[start:end]
    if offsets == "utf16":
        # UTF-16 은 2바이트가 한 단위 → 좌표에 2 를 곱해 바이트 위치로 바꾼다
        b = text.encode("utf-16-le", "surrogatepass")
        return b[start * 2:end * 2].decode("utf-16-le", "replace")
    return text.encode("utf-8", "replace")[start:end].decode("utf-8", "replace")


#------------------------------------------------------------------
# 값 가리기(화면 출력용)
#=> 앞 2글자만 남기고 * 로 바꾼다. --hide-values 일 때 결과표에 진짜 값이 찍히지 않게 한다.
#
# -in: v = 원래 값
#
# -out: str = 가린 값
# -out: error = 없음
#------------------------------------------------------------------
def redact(v):
    return v[:2] + "*" * max(len(v) - 2, 0)


#------------------------------------------------------------------
# info 명령 — 불러온 DLL 정보
#
# -in: args = argparse 결과
#
# -out: int = 종료 코드(0)
# -out: error = DLL 을 못 불러오면 MoaiPiiError 전파(main 에서 처리)
#------------------------------------------------------------------
def cmd_info(args):
    mp.load(args.lib)
    lb = mp.labels()
    print("라이브러리  : %s" % mp.lib_path())
    print("버전        : %s" % mp.version())
    print("ABI         : %d" % mp._lib.moaipii_abi_version())
    print("ko-pii 기준 : %s" % mp.kopii_basis())
    print("라벨        : %d개 (+ 별칭 %s)" % (len(lb["labels"]),
                                         ", ".join("%s→%s" % kv for kv in lb.get("aliases", {}).items())))
    return 0


#------------------------------------------------------------------
# labels 명령 — 지원 라벨 목록
#
# -in: args = argparse 결과(json 이면 원본 JSON)
#
# -out: int = 종료 코드(0)
# -out: error = DLL 을 못 불러오면 MoaiPiiError 전파
#------------------------------------------------------------------
def cmd_labels(args):
    mp.load(args.lib)
    lb = mp.labels()
    if args.json:
        print(json.dumps(lb, ensure_ascii=False, indent=1))
        return 0
    for name in lb["labels"]:
        print(name)
    for a, t in lb.get("aliases", {}).items():
        print("%s  (별칭 → %s)" % (a, t))
    return 0


#------------------------------------------------------------------
# scan 명령 — 본문 검사 결과를 표(또는 JSON)로
#=> 1) 본문을 읽고 라벨을 정한다
#    2) moaipii.scan 으로 검사(--offsets 단위 좌표)
#    3) 좌표로 값을 잘라 표로 찍는다 — 감싸개의 values 옵션 대신 직접 잘라 '좌표가 맞는지'도 보인다
#
# -in: args = argparse 결과
#
# -out: int = 종료 코드(0 = 성공, 검출이 0건이어도 0)
# -out: error = 옵션·본문 오류면 MoaiPiiError 전파
#------------------------------------------------------------------
def cmd_scan(args):
    mp.load(args.lib)
    text = read_text(args)
    labels = parse_labels(args.labels)
    t0 = time.perf_counter()
    r = mp.scan(text, labels, items=not args.no_items, offsets=args.offsets,
                unknown_labels="ignore" if args.ignore_unknown else "error", today=args.today)
    ms = (time.perf_counter() - t0) * 1000
    if args.json:
        print(json.dumps(r, ensure_ascii=False, indent=1))
        return 0
    print("본문 %d자 · 라벨 %d개 · %.2f ms · 좌표 단위 %s" % (r["chars"], len(labels), ms, args.offsets))
    if r.get("unknown"):
        print("모르는 라벨(무시함): %s" % ", ".join(r["unknown"]))
    if not r["counts"]:
        print("검출 없음")
        return 0
    print("\n[건수]")
    for k, v in sorted(r["counts"].items(), key=lambda kv: -kv[1]):
        print("  %-18s %d" % (k, v))
    if args.no_items:
        return 0
    print("\n[위치]")
    print("  %-18s %8s %8s  %s" % ("라벨", "start", "end", "값"))
    for it in r["items"]:
        v = slice_by(text, it["start"], it["end"], args.offsets)
        print("  %-18s %8d %8d  %s" % (it["label"], it["start"], it["end"],
                                       redact(v) if args.hide_values else v))
    return 0


#------------------------------------------------------------------
# mask 명령 — 검출된 자리를 가린 본문 출력
#=> 뒤에서부터 바꿔야 앞쪽 좌표가 밀리지 않는다(바꾼 길이가 원래와 달라도 안전).
#    style=star  : 같은 글자 수의 *
#    style=label : [PHONE] 처럼 라벨 이름
#    style=keep  : 앞 --keep 글자만 남기고 *
#
# -in: args = argparse 결과(style, keep, out)
#
# -out: int = 종료 코드(0)
# -out: error = MoaiPiiError 전파, 출력 파일을 못 쓰면 OSError
#------------------------------------------------------------------
def cmd_mask(args):
    mp.load(args.lib)
    text = read_text(args)
    r = mp.scan(text, parse_labels(args.labels))  # Python str 이라 char 좌표가 맞다
    out = text
    for it in reversed(r["items"]):
        s, e = it["start"], it["end"]
        if args.style == "label":
            rep = "[%s]" % it["label"]
        elif args.style == "keep":
            rep = out[s:s + args.keep] + "*" * max(e - s - args.keep, 0)
        else:
            rep = "*" * (e - s)
        out = out[:s] + rep + out[e:]
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="") as f:
            f.write(out)
        sys.stderr.write("가린 본문을 %s 에 썼습니다(%d곳).\n" % (args.out, len(r["items"])))
    else:
        sys.stdout.write(out if out.endswith("\n") else out + "\n")
    return 0


#------------------------------------------------------------------
# demo 실행 — 라벨별 예문이 기대대로 잡히는지
#=> 예문마다 그 라벨 하나만 켜서 검사하고, 잡힌 값이 기대값과 같은지 본다.
#
# -in: verbose = True 면 PASS 줄도 찍는다
#
# -out: (int, int) = (통과 수, 전체 수)
# -out: error = MoaiPiiError 전파
#------------------------------------------------------------------
def run_demo(verbose=True):
    ok = 0
    for label, sent, want in DEMO:
        r = mp.scan(sent, [label])
        got = [sent[it["start"]:it["end"]] for it in r["items"] if it["label"] == label]
        passed = got == [want]
        ok += passed
        if verbose or not passed:
            print("  [%s] %-18s %s  → %s" % ("PASS" if passed else "FAIL", label, sent,
                                             ", ".join(got) if got else "(없음)"))
    return ok, len(DEMO)


#------------------------------------------------------------------
# demo 명령
#
# -in: args = argparse 결과
#
# -out: int = 0(전부 통과) | 1(실패 있음)
# -out: error = MoaiPiiError 전파
#------------------------------------------------------------------
def cmd_demo(args):
    mp.load(args.lib)
    print("라벨별 예문 검사 (%s)" % mp.version())
    ok, n = run_demo()
    print("\n결과: %d / %d 통과" % (ok, n))
    return 0 if ok == n else 1


#------------------------------------------------------------------
# 시험 한 건 실행·기록
#=> 함수 하나를 돌려 예외 없이 True 를 돌려주면 통과로 본다.
#
# -in: name    = 시험 이름
# -in: fn      = 인자 없는 시험 함수(bool 반환)
# -in: results = (이름, 통과여부, 사유) 를 쌓을 목록
#
# -out: 없음
# -out: error = 없음(시험 중 예외는 실패로 기록)
#------------------------------------------------------------------
def check(name, fn, results):
    try:
        passed, why = fn(), ""
    except Exception as e:  # 시험 도구라 어떤 예외든 실패로 남긴다
        passed, why = False, "%s: %s" % (type(e).__name__, e)
    results.append((name, bool(passed), why))
    print("  [%s] %s%s" % ("PASS" if passed else "FAIL", name, ("  — " + why) if why else ""))


#------------------------------------------------------------------
# 기대한 MoaiPiiError 가 나는지
#
# -in: kind = 기대하는 오류 종류("arg" | "utf8")
# -in: fn   = 실행할 호출
#
# -out: bool = 그 종류의 MoaiPiiError 가 났으면 True
# -out: error = 다른 예외는 그대로 전파
#------------------------------------------------------------------
def raises(kind, fn):
    try:
        fn()
    except mp.MoaiPiiError as e:
        return e.kind == kind
    return False


#------------------------------------------------------------------
# selftest 명령 — 연동할 때 꼭 확인할 것들을 한 번에
#=> 1) 라벨별 예문(demo)
#    2) 좌표 단위 3종이 이모지 섞인 본문에서 같은 값을 가리키는지
#    3) 건수만 모드, 모르는 라벨 error/ignore, 깨진 UTF-8 error/replace, 본문 속 \0
#    4) 여러 스레드 동시 호출이 같은 결과를 내는지
#
# -in: args = argparse 결과
#
# -out: int = 0(전부 통과) | 1(실패 있음)
# -out: error = DLL 을 못 불러오면 MoaiPiiError 전파
#------------------------------------------------------------------
def cmd_selftest(args):
    mp.load(args.lib)
    print("moaipii selftest — %s\n  %s\n" % (mp.version(), mp.lib_path()))
    results = []

    print("[1] 라벨별 예문")
    ok, n = run_demo(verbose=False)
    results.append(("라벨별 예문 %d/%d" % (ok, n), ok == n, ""))
    print("  [%s] %d / %d" % ("PASS" if ok == n else "FAIL", ok, n))

    print("\n[2] 좌표 단위")
    emo = "홍길동 \U0001F600 010-1234-5678"
    # 이모지는 char 1 · utf16 2 · byte 4 칸이라 단위마다 좌표가 달라야 한다
    want = {"char": (6, 19), "utf16": (7, 20), "byte": (15, 28)}
    for unit, se in want.items():
        check("offsets=%-5s → %s" % (unit, se),
              lambda u=unit, se=se: (lambda it: (it["start"], it["end"]) == se
                                     and slice_by(emo, it["start"], it["end"], u) == "010-1234-5678")
              (mp.scan(emo, ["PHONE"], offsets=u)["items"][0]), results)

    print("\n[3] 옵션·오류")
    multi = "연락처 010-1234-5678, 메일 hong@example.co.kr, 또 010-9876-5432"
    check("items=False 면 건수만", lambda: (lambda r: r["counts"] == {"PHONE": 2, "EMAIL": 1}
                                          and not r.get("items"))
          (mp.scan(multi, ["PHONE", "EMAIL"], items=False)), results)
    check("values=True 면 원문 값", lambda: mp.scan(multi, ["EMAIL"], values=True)["items"][0]["value"]
          == "hong@example.co.kr", results)
    check("모르는 라벨 → arg 오류", lambda: raises("arg", lambda: mp.scan(multi, ["FAX"])), results)
    check("모르는 라벨 ignore → unknown 에 알림",
          lambda: mp.scan(multi, ["PHONE", "FAX"], unknown_labels="ignore").get("unknown") == ["FAX"], results)
    # 별칭은 ko-pii 호환 규칙상 '건수만' 별칭 이름으로 세고, 위치는 PRESCRIPTION_ID 를 켜야 나온다
    presc = "처방전 교부번호 202401150001 입니다."
    check("별칭 PRESCRIPTION → 건수만(위치 없음)", lambda: (lambda r: r["counts"] == {"PRESCRIPTION": 1}
                                                     and r["items"] == [])
          (mp.scan(presc, ["PRESCRIPTION"])), results)
    bad = "연락처 010-1234-5678 ".encode("utf-8") + b"\xff\xfe" + " 끝".encode("utf-8")
    check("깨진 UTF-8 → utf8 오류", lambda: raises("utf8", lambda: mp.scan(bad, ["PHONE"])), results)
    check("깨진 UTF-8 replace → 계속 검사",
          lambda: mp.scan(bad, ["PHONE"], invalid_utf8="replace")["counts"] == {"PHONE": 1}, results)
    nul = "앞\x00연락처 010-1234-5678"
    check("본문 속 \\0 에서 안 잘림", lambda: mp.scan(nul, ["PHONE"])["counts"] == {"PHONE": 1}, results)
    check("빈 본문", lambda: mp.scan("", ["PHONE"])["counts"] == {}, results)
    check("labels 비어 있어도 오류 없음", lambda: mp.scan(multi, [])["counts"] == {}, results)

    print("\n[4] 스레드 동시 호출")
    body = "\n".join(s for _, s, _ in DEMO) * 50
    alls = parse_labels("all")
    base = mp.scan(body, alls, items=False)["counts"]
    errs = []

    # 같은 본문을 여러 스레드가 동시에 검사해 기준 결과와 다르면 기록
    def worker():
        for _ in range(5):
            if mp.scan(body, alls, items=False)["counts"] != base:
                errs.append(1)
    ths = [threading.Thread(target=worker) for _ in range(8)]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    results.append(("8 스레드 × 5회 결과 일치", not errs, ""))
    print("  [%s] 8 스레드 × 5회 결과 일치" % ("PASS" if not errs else "FAIL"))

    fails = [r for r in results if not r[1]]
    print("\n결과: %d / %d 통과" % (len(results) - len(fails), len(results)))
    return 0 if not fails else 1


#------------------------------------------------------------------
# bench 명령 — 처리 속도 측정
#=> 같은 본문을 repeat 번 검사한다. threads 가 2 이상이면 나눠서 동시에 돌린다
#   (ctypes 호출 중엔 GIL 이 풀려 실제로 병렬로 돈다).
#
# -in: args = argparse 결과(repeat, threads)
#
# -out: int = 종료 코드(0)
# -out: error = MoaiPiiError 전파
#------------------------------------------------------------------
def cmd_bench(args):
    mp.load(args.lib)
    text = read_text(args)
    labels = parse_labels(args.labels)
    data = text.encode("utf-8")
    # 첫 호출은 정규식 준비 비용이 섞이므로 재기 전에 한 번 돌려 둔다
    counts = mp.scan(data, labels, items=False)["counts"]
    per = max(args.repeat // args.threads, 1)

    # 스레드 하나가 per 번 검사
    def worker():
        for _ in range(per):
            mp.scan(data, labels, items=False)
    t0 = time.perf_counter()
    ths = [threading.Thread(target=worker) for _ in range(args.threads)]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    sec = time.perf_counter() - t0
    n = per * args.threads
    mb = len(data) * n / 1048576
    print("본문 %.1f KB · 라벨 %d개 · %d회 × %d스레드" % (len(data) / 1024, len(labels), per, args.threads))
    print("총 %.3f s · 1회 평균 %.2f ms · 처리량 %.1f MB/s" % (sec, sec * 1000 / n * args.threads, mb / sec))
    print("건수: %s" % json.dumps(counts, ensure_ascii=False))
    return 0


#------------------------------------------------------------------
# 명령줄 해석기 만들기
#
# -in: 없음
#
# -out: argparse.ArgumentParser = 하위 명령이 붙은 해석기
# -out: error = 없음
#------------------------------------------------------------------
def build_parser():
    p = argparse.ArgumentParser(prog="moaipii_cli", description="moaipii DLL 시험 CLI 샘플")
    p.add_argument("--lib", help="moaipii.dll / libmoaipii.so 파일 또는 폴더(기본: MOAIPII_LIB → 감싸개 옆)")
    sub = p.add_subparsers(dest="cmd")
    sub.required = True

    sub.add_parser("info", help="불러온 DLL 경로·버전·ABI")
    sp = sub.add_parser("labels", help="지원 라벨 목록")
    sp.add_argument("--json", action="store_true", help="원본 JSON 으로")
    sub.add_parser("demo", help="라벨별 예문 20개 검출 확인")
    sub.add_parser("selftest", help="demo + 좌표·옵션·오류·스레드 시험")

    # scan·mask·bench 가 함께 쓰는 본문 입력 인자
    def add_input(sp):
        sp.add_argument("file", nargs="?", help="검사할 텍스트 파일('-' = 표준입력)")
        sp.add_argument("--text", help="파일 대신 이 문자열을 검사")
        sp.add_argument("--encoding", default="utf-8", help="파일 인코딩(기본 utf-8, EUC-KR 은 cp949)")
        sp.add_argument("--labels", default="all", help="켤 라벨(쉼표 구분, 기본 all)")

    sp = sub.add_parser("scan", help="본문 검사 결과 표")
    add_input(sp)
    sp.add_argument("--offsets", choices=["char", "utf16", "byte"], default="char", help="좌표 단위")
    sp.add_argument("--no-items", action="store_true", help="건수만")
    sp.add_argument("--hide-values", action="store_true", help="값을 앞 2글자만 보이고 가림")
    sp.add_argument("--ignore-unknown", action="store_true", help="모르는 라벨을 오류 대신 무시")
    sp.add_argument("--today", help="미래 생년월일 판단 기준일 YYYY-MM-DD")
    sp.add_argument("--json", action="store_true", help="DLL 결과 JSON 그대로")

    sp = sub.add_parser("mask", help="검출 자리를 가린 본문 출력")
    add_input(sp)
    sp.add_argument("--style", choices=["star", "label", "keep"], default="star", help="가리는 방식")
    sp.add_argument("--keep", type=int, default=3, help="style=keep 일 때 남길 앞 글자 수")
    sp.add_argument("--out", help="출력 파일(기본 표준출력, UTF-8)")

    sp = sub.add_parser("bench", help="처리 속도 측정")
    add_input(sp)
    sp.add_argument("--repeat", type=int, default=20, help="총 검사 횟수(기본 20)")
    sp.add_argument("--threads", type=int, default=1, help="동시 스레드 수(기본 1)")
    return p


#------------------------------------------------------------------
# 진입점
#=> 명령을 골라 실행하고, 라이브러리 오류는 kind 와 함께 한 줄로 알린다.
#
# -in: argv = 명령줄 인자(None 이면 sys.argv)
#
# -out: int = 종료 코드(0 성공 · 1 시험 실패 · 2 인자 오류 · 4 라이브러리 오류)
# -out: error = 없음(MoaiPiiError 는 잡아서 종료 코드로)
#------------------------------------------------------------------
def main(argv=None):
    utf8_console()
    args = build_parser().parse_args(argv)
    # 이후 모든 명령의 mp.load(args.lib) 가 같은 DLL 을 쓰도록 여기서 한 번 정한다
    args.lib = default_lib(args.lib)
    cmds = {"info": cmd_info, "labels": cmd_labels, "scan": cmd_scan, "mask": cmd_mask,
            "demo": cmd_demo, "selftest": cmd_selftest, "bench": cmd_bench}
    try:
        return cmds[args.cmd](args)
    except mp.MoaiPiiError as e:
        sys.stderr.write("[moaipii 오류] kind=%s code=%s : %s\n" % (e.kind, e.code, e))
        return 4


if __name__ == "__main__":
    sys.exit(main())
