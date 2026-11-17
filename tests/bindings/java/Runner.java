import moaipii.Json;
import moaipii.MoaiPii;

import java.io.PrintStream;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/**
 * 감싸개 공통 시험 — Java 실행기.
 * cases.json 의 사례를 Java 감싸개(bindings/java/moaipii)로 돌려 한 줄에 한 사례씩 JSON 을 낸다.
 *  1) 본문: 올바른 UTF-8 이면 String 으로 scan, text_u16 사례는 char[] 로 String 을 만들고,
 *     깨진 UTF-8 이면 scanBytes 로 바이트를 그대로
 *  2) values 가 있고 좌표가 이 언어의 단위(utf16)면 substring 이 값과 같은지(slice_ok)
 *  3) threads 사례는 같은 MoaiPii 를 여러 스레드가 함께 써서 결과가 같은지(threads_ok)
 *  4) fake_abi 는 MOAIPII_FAKE_LIB 를 불러 abi 오류가 나는지
 *
 * 사용: java -cp jna.jar:out Runner cases.json   (라이브러리는 환경변수 MOAIPII_LIB)
 */
public final class Runner {
    static final String LANG = "java";

    /** 사례 한 번 검사 → 결과 JSON 조각 */
    @SuppressWarnings("unchecked")
    static String once(MoaiPii pii, Object text, Map<String, Object> c) {
        Map<String, Object> o = (Map<String, Object>) c.get("opts");
        MoaiPii.Options op = new MoaiPii.Options();
        op.items = (Boolean) o.get("items");
        op.values = (Boolean) o.get("values");
        op.offsets = (String) o.get("offsets");
        op.invalidUtf8 = (String) o.get("invalid_utf8");
        op.unknownLabels = (String) o.get("unknown_labels");
        op.today = (String) o.get("today");
        List<String> labels = new ArrayList<String>();
        for (Object x : (List<Object>) c.get("labels")) labels.add((String) x);
        try {
            MoaiPii.Result r = text instanceof String
                    ? pii.scan((String) text, labels, op)
                    : pii.scanBytes(((byte[]) text).clone(), labels, op);  // scanBytes 는 넘긴 배열을 지우므로 사본
            StringBuilder b = new StringBuilder("\"code\":0,\"counts\":{");
            boolean first = true;
            for (Map.Entry<String, Long> e : r.counts.entrySet()) {
                if (!first) b.append(',');
                b.append(Json.quote(e.getKey())).append(':').append(e.getValue());
                first = false;
            }
            b.append("},\"items\":[");
            for (int k = 0; k < r.items.size(); k++) {
                MoaiPii.Item it = r.items.get(k);
                if (k > 0) b.append(',');
                b.append("{\"label\":").append(Json.quote(it.label)).append(",\"start\":").append(it.start)
                 .append(",\"end\":").append(it.end);
                if (it.value != null) b.append(",\"value\":").append(Json.quote(it.value));
                b.append('}');
            }
            b.append("],\"unknown\":[");
            for (int k = 0; k < r.unknown.size(); k++) {
                if (k > 0) b.append(',');
                b.append(Json.quote(r.unknown.get(k)));
            }
            b.append(']');
            // 이 언어의 자르기 단위(utf16)면 원래 String 을 잘라 값과 맞춰 본다
            if (text instanceof String && op.values && "utf16".equals(op.offsets)) {
                boolean ok = true;
                for (MoaiPii.Item it : r.items) ok &= ((String) text).substring(it.start, it.end).equals(it.value);
                b.append(",\"slice_ok\":").append(ok);
            }
            return b.toString();
        } catch (MoaiPii.MoaiPiiException e) {
            return "\"code\":" + e.code + ",\"kind\":" + Json.quote(e.kind);
        }
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        PrintStream out = new PrintStream(System.out, true, "UTF-8");
        Map<String, Object> doc = (Map<String, Object>) Json.parse(new String(Files.readAllBytes(Paths.get(args[0])), StandardCharsets.UTF_8));
        MoaiPii pii = MoaiPii.load();
        for (Object co : (List<Object>) doc.get("cases")) {
            Map<String, Object> c = (Map<String, Object>) co;
            if (!((List<Object>) c.get("langs")).contains(LANG)) continue;
            String head = "{\"name\":" + Json.quote((String) c.get("name")) + ",\"lang\":\"" + LANG + "\",";
            if (Boolean.TRUE.equals(c.get("fake_abi"))) {
                try {
                    MoaiPii.load(System.getenv("MOAIPII_FAKE_LIB"));
                    out.println(head + "\"code\":0}");
                } catch (MoaiPii.MoaiPiiException e) {
                    out.println(head + "\"code\":-1,\"kind\":" + Json.quote(e.kind) + "}");
                }
                continue;
            }
            Object text;
            if (c.containsKey("text_u16")) {
                List<Object> u = (List<Object>) c.get("text_u16");
                char[] cs = new char[u.size()];
                for (int k = 0; k < cs.length; k++) cs[k] = (char) ((Number) u.get(k)).intValue();
                text = new String(cs);
            } else {
                byte[] one = Base64.getDecoder().decode((String) c.get("text_b64"));
                int rep = c.containsKey("repeat") ? ((Number) c.get("repeat")).intValue() : 1;
                byte[] raw = new byte[one.length * rep];
                for (int k = 0; k < rep; k++) System.arraycopy(one, 0, raw, k * one.length, one.length);
                // 올바른 UTF-8 이면 String, 깨졌으면 byte[] 그대로
                try {
                    text = StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                            .decode(ByteBuffer.wrap(raw)).toString();
                } catch (CharacterCodingException e) {
                    text = raw;
                }
            }
            String first = once(pii, text, c);
            String line = head + first;
            if (c.containsKey("threads")) {
                int n = ((Number) c.get("threads")).intValue();
                ExecutorService ex = Executors.newFixedThreadPool(n);
                List<Future<String>> fs = new ArrayList<Future<String>>();
                final Object t = text;
                for (int k = 0; k < n; k++) fs.add(ex.submit(() -> once(pii, t, c)));
                boolean ok = true;
                for (Future<String> f : fs) ok &= f.get().equals(first);
                ex.shutdown();
                line += ",\"threads_ok\":" + ok;
            }
            out.println(line + "}");
        }
    }
}
