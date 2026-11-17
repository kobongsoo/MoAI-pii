# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 감싸개 공통 시험 — 정답과 비교
#=> 언어별 실행기가 낸 결과(out/<언어>.jsonl)를 cases.json 의 정답과 맞춰 본다.
#    1) 그 언어에서 돌려야 할 사례가 빠짐없이 나왔는가
#    2) code·kind·counts·items(label·start·end, 정답에 value 가 있으면 value 도)·unknown 이 같은가
#    3) slice_ok 가 false 면 실패(그 언어로 자른 값이 라이브러리가 준 값과 다르다)
#    4) threads 사례는 threads_ok 가 true 여야 한다
#
#   사용: python -I tests/bindings/compare.py <cases.json> <out/py.jsonl> [<out/rs.jsonl> …]
#   종료 코드: 0 = 전부 같음, 1 = 다른 것이 있음
#------------------------------------------------------------------
import io
import json
import os
import sys


#------------------------------------------------------------------
# 결과 한 줄을 정답과 비교
#
# -in: exp = 사례의 expect
# -in: got = 실행기 결과 한 줄
# -in: c   = 사례 전체(threads 여부를 본다)
#
# -out: list[str] = 다른 점들(비면 같음)
# -out: error = 없음
#------------------------------------------------------------------
def diff(exp, got, c):
    bad = []
    if got.get("code") != exp["code"]:
        bad.append("code %r ≠ %r" % (got.get("code"), exp["code"]))
    if "kind" in exp and got.get("kind") != exp["kind"]:
        bad.append("kind %r ≠ %r" % (got.get("kind"), exp["kind"]))
    if exp["code"] == 0:
        if (got.get("counts") or {}) != exp.get("counts", {}):
            bad.append("counts %r ≠ %r" % (got.get("counts"), exp.get("counts")))
        want = exp.get("items", [])
        have = got.get("items") or []
        if len(have) != len(want):
            bad.append("items 수 %d ≠ %d" % (len(have), len(want)))
        for k, (h, w) in enumerate(zip(have, want)):
            keys = ["label", "start", "end"] + (["value"] if "value" in w else [])
            if any(h.get(x) != w.get(x) for x in keys):
                bad.append("items[%d] %r ≠ %r" % (k, {x: h.get(x) for x in keys}, w))
        if (got.get("unknown") or []) != exp.get("unknown", []):
            bad.append("unknown %r ≠ %r" % (got.get("unknown"), exp.get("unknown")))
    if got.get("slice_ok") is False:
        bad.append("이 언어로 자른 값이 라이브러리 값과 다르다(slice_ok=false)")
    if c.get("threads") and got.get("threads_ok") is not True:
        bad.append("동시 호출 결과가 서로 다르다(threads_ok=%r)" % got.get("threads_ok"))
    return bad


#------------------------------------------------------------------
# 진입점
#
# -in: 없음(명령줄)
#
# -out: 없음(종료 코드)
# -out: error = 다른 것이 있으면 종료 코드 1
#------------------------------------------------------------------
def main():
    cases = {c["name"]: c for c in json.load(io.open(sys.argv[1], encoding="utf-8"))["cases"]}
    w = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", newline="\n")
    fails = 0
    for path in sys.argv[2:]:
        lang = os.path.splitext(os.path.basename(path))[0]
        got = {}
        for line in io.open(path, encoding="utf-8"):
            line = line.strip()
            if line:
                r = json.loads(line)
                got[r["name"]] = r
        n_ok = 0
        for name, c in cases.items():
            if lang not in c["langs"]:
                continue
            if name not in got:
                w.write("  [%s] %s: 결과 없음\n" % (lang, name))
                fails += 1
                continue
            bad = diff(c["expect"], got[name], c)
            if bad:
                fails += 1
                w.write("  [%s] %s: %s\n" % (lang, name, " / ".join(bad)))
            else:
                n_ok += 1
        total = sum(1 for c in cases.values() if lang in c["langs"])
        w.write("  %-8s %d/%d 같음\n" % (lang, n_ok, total))
    w.flush()
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
