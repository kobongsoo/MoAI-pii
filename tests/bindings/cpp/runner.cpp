// 감싸개 공통 시험 — C++ 실행기
//=> cases.json 의 사례를 C++ 감싸개(moaipii.hpp)로 돌려 한 줄에 한 사례씩 JSON 을 낸다.
//    1) 본문은 바이트 그대로(std::string)
//    2) values 가 있고 좌표가 이 언어의 단위(byte)면 substr 가 값과 같은지(slice_ok)
//    3) threads 사례는 같은 Lib 을 여러 스레드가 함께 써서 결과가 같은지(threads_ok)
//    4) fake_abi 는 MOAIPII_FAKE_LIB 를 불러 abi 오류가 나는지(동적 로드형만)
//   같은 소스를 두 번 빌드한다: MOAIPII_DYNAMIC 을 켜면 "cpp"(동적 로드형), 끄면 "cpp-link"(링크형).
//
//   사용: runner <cases.json>   (동적 로드형은 환경변수 MOAIPII_LIB 로 라이브러리를 찾는다)
#include "moaipii.hpp"

#include <fstream>
#include <iostream>
#include <sstream>
#include <thread>

#ifdef MOAIPII_DYNAMIC
static const char* LANG = "cpp";
#else
static const char* LANG = "cpp-link";
#endif

using moaipii::detail::Value;

// base64 → 바이트 (표준 알파벳)
static std::string b64(const std::string& s) {
    auto val = [](char c) -> int {
        if (c >= 'A' && c <= 'Z') return c - 'A';
        if (c >= 'a' && c <= 'z') return c - 'a' + 26;
        if (c >= '0' && c <= '9') return c - '0' + 52;
        if (c == '+') return 62;
        if (c == '/') return 63;
        return -1;
    };
    std::string out;
    unsigned acc = 0;
    int bits = 0;
    for (char c : s) {
        int v = val(c);
        if (v < 0) continue;
        acc = (acc << 6) | unsigned(v);
        bits += 6;
        if (bits >= 8) { bits -= 8; out += char((acc >> bits) & 0xFF); acc &= (1u << bits) - 1; }
    }
    return out;
}

// 사례 한 번 검사 → 결과 JSON 조각
static std::string once(const moaipii::Lib& pii, const std::string& text, const Value& c, bool* slice_ok) {
    const Value& o = *c.get("opts");
    moaipii::Options op;
    op.items = std::get<bool>(o.get("items")->v);
    op.values = std::get<bool>(o.get("values")->v);
    op.offsets = *o.get("offsets")->str();
    op.invalid_utf8 = *o.get("invalid_utf8")->str();
    op.unknown_labels = *o.get("unknown_labels")->str();
    if (auto t = o.get("today")->str()) op.today = *t;
    std::vector<std::string> labels;
    for (auto& x : *c.get("labels")->arr()) labels.push_back(*x.str());
    try {
        auto r = pii.scan(text, labels, op);
        std::string j = "\"code\":0,\"counts\":{";
        bool first = true;
        for (auto& [k, n] : r.counts) { if (!first) j += ','; j += moaipii::detail::quote(k) + ":" + std::to_string(n); first = false; }
        j += "},\"items\":[";
        bool ok = true;
        for (std::size_t k = 0; k < r.items.size(); ++k) {
            auto& it = r.items[k];
            if (k) j += ',';
            j += "{\"label\":" + moaipii::detail::quote(it.label) + ",\"start\":" + std::to_string(it.start) +
                 ",\"end\":" + std::to_string(it.end);
            if (it.value) j += ",\"value\":" + moaipii::detail::quote(*it.value);
            j += "}";
            // 이 언어의 자르기 단위(byte)면 원래 문자열을 잘라 값과 맞춰 본다
            if (op.values && op.offsets == "byte")
                ok = ok && it.value && it.end <= text.size() && text.substr(it.start, it.end - it.start) == *it.value;
        }
        j += "],\"unknown\":[";
        for (std::size_t k = 0; k < r.unknown.size(); ++k) { if (k) j += ','; j += moaipii::detail::quote(r.unknown[k]); }
        j += "]";
        if (slice_ok && op.values && op.offsets == "byte") *slice_ok = ok;
        return j;
    } catch (const moaipii::error& e) {
        return "\"code\":" + std::to_string(e.code) + ",\"kind\":" + moaipii::detail::quote(e.kind);
    }
}

int main(int argc, char** argv) {
    if (argc < 2) { std::cerr << "사용: runner <cases.json>\n"; return 2; }
    std::ifstream f(argv[1], std::ios::binary);
    std::stringstream ss;
    ss << f.rdbuf();
    Value doc = moaipii::detail::Parser(ss.str()).parse();
    moaipii::Lib pii = moaipii::Lib::load();
    for (auto& c : *doc.get("cases")->arr()) {
        bool mine = false;
        for (auto& l : *c.get("langs")->arr()) if (*l.str() == LANG) mine = true;
        if (!mine) continue;
        std::string head = "{\"name\":" + moaipii::detail::quote(*c.get("name")->str()) + ",\"lang\":\"" + LANG + "\",";
        if (auto fa = c.get("fake_abi")) {
            if (std::get<bool>(fa->v)) {
                const char* fake = std::getenv("MOAIPII_FAKE_LIB");
                try {
                    moaipii::Lib::load(fake ? fake : "");
                    std::cout << head << "\"code\":0}\n";
                } catch (const moaipii::error& e) {
                    std::cout << head << "\"code\":-1,\"kind\":" << moaipii::detail::quote(e.kind) << "}\n";
                }
                continue;
            }
        }
        std::string one = b64(*c.get("text_b64")->str());
        std::size_t rep = c.get("repeat") ? static_cast<std::size_t>(c.get("repeat")->num()) : 1;
        std::string text;
        text.reserve(one.size() * rep);
        for (std::size_t k = 0; k < rep; ++k) text += one;
        bool slice_ok = true, has_slice = false;
        {
            const Value& o = *c.get("opts");
            has_slice = std::get<bool>(o.get("values")->v) && *o.get("offsets")->str() == "byte";
        }
        std::string first = once(pii, text, c, &slice_ok);
        std::string line = head + first;
        if (has_slice && first.rfind("\"code\":0", 0) == 0) line += std::string(",\"slice_ok\":") + (slice_ok ? "true" : "false");
        if (auto th = c.get("threads")) {
            int n = static_cast<int>(th->num());
            std::vector<std::string> res(static_cast<std::size_t>(n));
            std::vector<std::thread> ts;
            for (int k = 0; k < n; ++k) ts.emplace_back([&, k] { res[static_cast<std::size_t>(k)] = once(pii, text, c, nullptr); });
            for (auto& t : ts) t.join();
            bool ok = true;
            for (auto& r : res) ok = ok && r == first;
            line += std::string(",\"threads_ok\":") + (ok ? "true" : "false");
        }
        std::cout << line << "}\n";
    }
    return 0;
}
