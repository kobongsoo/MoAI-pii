# MoAI-pii — 한국어 개인정보(PII) 검출 라이브러리

본문 텍스트에서 주민등록번호·외국인등록번호·카드·사업자/법인등록번호·전화·이메일·계좌·여권·IP·URL·
차량번호·국적·주소·운전면허·건강보험·처방전·EDI 약품·사건번호·PNU 를 찾아 **위치와 건수**를 돌려준다.
검출 알고리즘은 ko-pii 1.15.2 를 옮겨 적은 것에 MpowerClassify 운영에서 찾은 보강(의도적 차이 A·A-2·B·G·H·I·J,
`doc/ko-pii 차이점.html`)을 더한 것이다.

- Windows `moaipii.dll` · Linux `libmoaipii.so`(CentOS 7 / glibc 2.17 에서 돈다), 외부 .so 의존 없음
- C 진입점 6개(`include/moaipii.h`) — 본문은 (포인터, 길이), 옵션·결과는 UTF-8 JSON
- 감싸개: Python(ctypes) · Rust(libloading) · Java(JNA) · PHP(FFI) · C++(헤더 하나)
- 설계: `plan/MoAI-pii-개인정보검출-라이브러리분리-설계-20261117.html`

## 구성

| 경로 | 내용 |
|---|---|
| `crates/moaipii` | 본체(검출기·정규화·겹침 해소·좌표 단위 변환) + 단위·골든 시험 |
| `crates/moaipii-ffi` | C 진입점 → `moaipii.dll` / `libmoaipii.so` |
| `crates/moaipii-cli` | 시험 도구 `mp` — 텍스트 파일을 C 진입점 그대로 검사 |
| `include/moaipii.h` | C 헤더 |
| `bindings/{python,rust,java,php,cpp}` | 언어별 감싸개 정본 |
| `tests/bindings` | 감싸개 공통 시험(`cases.json` 하나로 다섯 언어 비교) |
| `tools` | 골든 정답표·정규화표 생성기(ko-pii 필요, 개발 전용) |

## 빌드

```bash
bash scripts/win-build.sh      # Windows: 시험 → dll·mp.exe → 감싸개 공통 시험 → dist/*.zip
bash scripts/linux-build.sh    # 빌드서버(CentOS 7)에서 .so·mp → dist/*.tgz (scripts/server.env 필요)
```

## 시험 도구 mp

```bash
mp version
mp labels
mp scan 본문.txt --labels RRN,PHONE --offsets byte --values
mp scan 큰본문.txt --no-items --time
```

## 좌표 단위

언어마다 문자열을 세는 단위가 달라, 감싸개가 자기 언어의 단위를 요청한다(`offsets`).

| 언어 | 기본 | 자르기 |
|---|---|---|
| Python | `char` | `text[s:e]` |
| Java | `utf16` | `text.substring(s, e)` |
| PHP · C++ | `byte` | `substr($t, s, e-s)` · `t.substr(s, e-s)` |
| Rust 감싸개 | `char` | (`byte` 로 받으면 `&t[s..e]`) |

자세한 사용법은 `doc/moaipii-연동설명서.html`.
