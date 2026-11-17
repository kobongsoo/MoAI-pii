# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 감싸개 공통 시험 — Python 실행기
#=> cases.json 의 사례를 Python 감싸개(bindings/python/moaipii.py)로 돌려
#   결과를 한 줄에 한 사례씩 JSON 으로 낸다. 비교는 compare.py 가 한다.
#    1) 본문: 올바른 UTF-8 이면 str 로, 짝 없는 서로게이트 사례는 text_u16 로 str 을 만들고,
#       깨진 UTF-8 이면 bytes 그대로 넘긴다
#    2) values 가 있고 좌표가 이 언어의 단위(char)면 text[start:end] 가 값과 같은지 본다(slice_ok)
#    3) threads 가 있으면 같은 사례를 동시에 돌려 결과가 모두 같은지 본다(threads_ok)
#    4) fake_abi 는 MOAIPII_FAKE_LIB 를 불러 abi 오류가 나는지 본다
#
#   사용: python -I tests/bindings/runner.py <cases.json> <bindings/python 폴더>
#------------------------------------------------------------------
import base64
import io
import json
import os
import subprocess
import sys
import threading

LANG = "py"


#------------------------------------------------------------------
# 사례 하나 돌리기
#
# -in: m = moaipii 모듈
# -in: c = 사례(dict)
#
# -out: dict = 결과 한 줄({name, code, kind?, counts?, items?, unknown?, slice_ok?, threads_ok?})
# -out: error = 없음(감싸개 예외는 결과의 code·kind 로 바꾼다)
#------------------------------------------------------------------
def run_case(m, c):
    out = {"name": c["name"], "lang": LANG}
    if c.get("fake_abi"):
        # 감싸개는 한 번 불러오면 그것을 쓰므로, 가짜 라이브러리는 새 프로세스에서 불러 본다
        code = ("import sys; sys.path.insert(0, %r); import moaipii\n"
                "try:\n    moaipii.load(%r); print('ok')\n"
                "except moaipii.MoaiPiiError as e: print(e.kind)\n") % (os.path.dirname(m.__file__), os.environ["MOAIPII_FAKE_LIB"])
        r = subprocess.run([sys.executable, "-I", "-c", code], capture_output=True, text=True)
        kind = r.stdout.strip()
        out.update({"code": -1, "kind": kind} if kind != "ok" else {"code": 0})
        return out
    if "text_u16" in c:
        # UTF-16 코드 단위 → str (짝 없는 서로게이트도 그대로 들어간다)
        text = "".join(chr(u) for u in c["text_u16"])
    else:
        raw = base64.b64decode(c["text_b64"]) * int(c.get("repeat", 1))
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            text = raw
    o = c["opts"]

    # 사례 한 번 검사 → 결과 dict 또는 오류
    def once():
        try:
            r = m.scan(text, c["labels"], items=o["items"], values=o["values"], offsets=o["offsets"],
                       invalid_utf8=o["invalid_utf8"], unknown_labels=o["unknown_labels"], today=o["today"])
            return {"code": 0, "counts": r["counts"], "items": r["items"], "unknown": r.get("unknown", [])}
        except m.MoaiPiiError as e:
            return {"code": e.code, "kind": e.kind}

    first = once()
    out.update(first)
    # 이 언어의 자르기 단위(char)로 받은 좌표면 원래 본문을 잘라 값과 맞춰 본다
    if first["code"] == 0 and o["values"] and o["offsets"] == "char" and isinstance(text, str):
        out["slice_ok"] = all(text[i["start"]:i["end"]] == i.get("value") for i in first["items"])
    if c.get("threads"):
        res = [None] * int(c["threads"])

        # 스레드 하나가 같은 사례를 돌려 자리에 넣는다
        def work(k):
            res[k] = once()
        ts = [threading.Thread(target=work, args=(k,)) for k in range(len(res))]
        for t in ts:
            t.start()
        for t in ts:
            t.join()
        out["threads_ok"] = all(r == first for r in res)
    return out


#------------------------------------------------------------------
# 진입점
#
# -in: 없음(명령줄: cases.json, 감싸개 폴더)
#
# -out: 없음(표준출력에 JSON 줄들)
# -out: error = 라이브러리를 못 불러오면 예외로 끝난다
#------------------------------------------------------------------
def main():
    cases = json.load(io.open(sys.argv[1], encoding="utf-8"))["cases"]
    sys.path.insert(0, sys.argv[2])
    import moaipii
    moaipii.load()
    w = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", newline="\n")
    for c in cases:
        if LANG in c["langs"]:
            w.write(json.dumps(run_case(moaipii, c), ensure_ascii=False) + "\n")
    w.flush()


if __name__ == "__main__":
    main()
