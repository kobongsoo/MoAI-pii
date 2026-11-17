# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 감싸개 공통 시험 사례 만들기 → tests/bindings/cases.json
#=> 다섯 감싸개(Python·Rust·Java·PHP·C++)가 같은 사례 파일을 읽어 같은 답을 내는지 본다
#   (설계서 §5-5). 정답은 사람이 손으로 셈한 값이다 — 라이브러리로 뽑으면 라이브러리가
#   틀려도 시험이 통과하므로 그렇게 하지 않는다.
#   본문은 바이트 그대로 base64 로 적는다(\0 · 깨진 UTF-8 을 담으려고).
#   짝 없는 서로게이트처럼 바이트로 못 적는 본문은 UTF-16 코드 단위 목록(text_u16)으로 적는다.
#
#   사용: python tests/bindings/make_cases.py          (cases.json 새로 쓰기)
#         python tests/bindings/make_cases.py --check  (지금 파일과 같은지만 보기)
#------------------------------------------------------------------
import base64
import io
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "cases.json")

# 모든 언어 / 바이트를 못 넘기는 사례에 쓸 언어 묶음
ALL = ["py", "rs", "java", "php", "cpp", "cpp-link"]


#------------------------------------------------------------------
# 체크섬이 맞는 주민등록번호 만들기
#=> 생년월일·성별 자리는 주고 뒤 5자리는 0 으로 채운 뒤, 마지막 검증 숫자를 계산해 붙인다.
#    1) 앞 12자리 = YYMMDD + 성별 + 00000
#    2) 검증 숫자 = (11 - Σ(자리×가중치) mod 11) mod 10
#
# -in: front = 앞 6자리(YYMMDD)
# -in: g     = 성별 자리(1~4)
#
# -out: str = "YYMMDD-G00000C" (C = 검증 숫자)
# -out: error = 없음
#------------------------------------------------------------------
def rrn(front, g):
    w = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5]
    s = "%s%d00000" % (front, g)
    # 주민번호 검증 숫자 규칙(행정안전부 공개 산식)
    chk = (11 - sum(int(a) * b for a, b in zip(s, w)) % 11) % 10
    return "%s-%s%d" % (front, s[6:], chk)


#------------------------------------------------------------------
# 사례 하나 만들기
#=> 바이트 본문과 옵션·정답을 받아 cases.json 의 한 항목으로 만든다.
#
# -in: name   = 사례 이름
# -in: data   = 본문 bytes (text_u16 를 줄 때는 None)
# -in: labels = 켤 라벨
# -in: opts   = 옵션(offsets 등 — 언어 기본값 차이가 끼지 않게 늘 적는다)
# -in: expect = 정답 {code, kind?, counts?, items?, unknown?}
# -in: langs  = 돌릴 언어(기본 전부)
# -in: extra  = 그 밖의 칸(repeat·threads·text_u16·fake_abi)
#
# -out: dict = 사례
# -out: error = 없음
#------------------------------------------------------------------
def case(name, data, labels, opts, expect, langs=None, **extra):
    c = {"name": name, "labels": labels, "opts": opts, "expect": expect, "langs": langs or ALL}
    if data is not None:
        c["text_b64"] = base64.b64encode(data).decode("ascii")
    c.update(extra)
    return c


#------------------------------------------------------------------
# 전체 사례 목록
#
# -in: 없음
#
# -out: list[dict]
# -out: error = 없음
#------------------------------------------------------------------
def cases():
    emoji = "홍길동 😀 010-1234-5678".encode("utf-8")
    phone = {"label": "PHONE", "value": "010-1234-5678"}
    base = {"items": True, "values": True, "invalid_utf8": "error", "unknown_labels": "error", "today": None}
    o = lambda **k: dict(base, **k)  # 기본 옵션에서 몇 칸만 바꾼 것
    born2030 = rrn("300101", 3)
    out = [
        # 설계서 §5-2 — 이모지 앞뒤 좌표가 단위마다 맞는가
        case("좌표_char", emoji, ["PHONE"], o(offsets="char"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=6, end=19)]}),
        case("좌표_utf16", emoji, ["PHONE"], o(offsets="utf16"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=7, end=20)]}),
        case("좌표_byte", emoji, ["PHONE"], o(offsets="byte"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=15, end=28)]}),
        # 본문 중간 \0 뒤도 본다 — "head\0"(5) + "연락처 "(10) = 15
        case("NUL_뒤", b"head\x00" + "연락처 010-1234-5678".encode("utf-8"), ["PHONE"], o(offsets="byte"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=15, end=28)]}),
        # 깨진 UTF-8 — 오류, 또는 고쳐서 원래 바이트 좌표(FF 20 = 2, "전화 " = 7 → 9)
        case("깨진UTF8_오류", b"\xff " + "전화 010-1234-5678".encode("utf-8"), ["PHONE"], o(offsets="byte"),
             {"code": 2, "kind": "utf8"}),
        case("깨진UTF8_고침", b"\xff " + "전화 010-1234-5678".encode("utf-8"), ["PHONE"],
             o(offsets="byte", invalid_utf8="replace"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=9, end=22)]}),
        # 짝 없는 서로게이트 — 감싸개가 "?" 한 글자로 바꿔 넘긴다(바이트로 못 적어 text_u16)
        case("짝없는_서로게이트", None, ["PHONE"], o(offsets="char"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=2, end=15)]},
             langs=["py", "java"], text_u16=[0xD800, 0x20] + [ord(c) for c in "010-1234-5678"]),
        # 라벨 간 겹침 — 사업자번호 모양 전화번호는 사업자번호 하나로만 센다
        case("겹침_해소", "사업자 022-00-76422 연락 010-1234-5678".encode("utf-8"), ["BUSINESS_REG", "PHONE"],
             o(offsets="byte"),
             {"code": 0, "counts": {"BUSINESS_REG": 1, "PHONE": 1},
              "items": [{"label": "BUSINESS_REG", "start": 10, "end": 22, "value": "022-00-76422"},
                        dict(phone, start=30, end=43)]}),
        case("빈_본문", b"", ["PHONE"], o(offsets="byte"), {"code": 0, "counts": {}, "items": []}),
        case("모르는_라벨_오류", b"x", ["NOPE"], o(offsets="byte"), {"code": 1, "kind": "arg"}),
        case("모르는_라벨_무시", b"010-1234-5678", ["NOPE", "PHONE"], o(offsets="byte", unknown_labels="ignore"),
             {"code": 0, "counts": {"PHONE": 1}, "items": [dict(phone, start=0, end=13)], "unknown": ["NOPE"]}),
        case("건수만", "메일 a.b@example.co.kr".encode("utf-8"), ["EMAIL"], o(offsets="byte", items=False),
             {"code": 0, "counts": {"EMAIL": 1}, "items": []}),
        # 기준일 — 2030 년생은 2026 기준으로 버리고 2031 기준으로 잡는다
        case("기준일_2026", ("주민등록번호 " + born2030).encode("utf-8"), ["RRN"],
             o(offsets="byte", values=False, today="2026-11-17"), {"code": 0, "counts": {}, "items": []}),
        case("기준일_2031", ("주민등록번호 " + born2030).encode("utf-8"), ["RRN"],
             o(offsets="byte", values=False, today="2031-01-01"),
             {"code": 0, "counts": {"RRN": 1}, "items": [{"label": "RRN", "start": 19, "end": 33}]}),
        # 5MB 쯤 — 건수만 받는 경로(주소록 같은 큰 문서)
        case("큰_본문_건수만", "연락처 010-1234-5678\n".encode("utf-8"), ["PHONE"], o(offsets="byte", items=False),
             {"code": 0, "counts": {"PHONE": 200000}, "items": []}, repeat=200000),
        # 8 스레드 동시 호출 — 결과가 한 스레드일 때와 같다(PHP·Python 은 실행기 구조상 빼거나 순차)
        case("동시_호출", "사업자 022-00-76422 연락 010-1234-5678".encode("utf-8"), ["BUSINESS_REG", "PHONE"],
             o(offsets="byte", values=False),
             {"code": 0, "counts": {"BUSINESS_REG": 1, "PHONE": 1},
              "items": [{"label": "BUSINESS_REG", "start": 10, "end": 22}, {"label": "PHONE", "start": 30, "end": 43}]},
             langs=["py", "rs", "java", "cpp", "cpp-link"], threads=8),
        # ABI 가 다른 가짜 라이브러리는 불러오기에서 막힌다(링크형 C++ 은 불러오기가 없어 뺀다)
        case("ABI_다름", b"", [], o(), {"code": -1, "kind": "abi"},
             langs=["py", "rs", "java", "php", "cpp"], fake_abi=True),
    ]
    return out


if __name__ == "__main__":
    doc = {"_note": "tests/bindings/make_cases.py 가 만든다 — 손으로 고치지 말 것", "cases": cases()}
    body = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"
    if "--check" in sys.argv:
        cur = io.open(OUT, encoding="utf-8").read() if os.path.isfile(OUT) else ""
        if cur != body:
            raise SystemExit("cases.json 이 make_cases.py 와 다릅니다 — 다시 만드세요")
        print("cases.json 최신")
    else:
        io.open(OUT, "w", encoding="utf-8", newline="\n").write(body)
        print("%s (%d 사례)" % (OUT, len(doc["cases"])))
