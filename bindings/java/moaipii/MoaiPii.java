package moaipii;

import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;

/**
 * moaipii.dll / libmoaipii.so 의 Java 감싸개 (JNA, Java 8 이상).
 *
 * <p>정본: MoAI-pii/bindings/java/moaipii — 쓰는 프로젝트는 이 파일들을 복사해 둔다.
 * 필요한 의존: {@code net.java.dev.jna:jna:5.14.0} (이상). 그 밖에는 없다.
 *
 * <pre>{@code
 * MoaiPii pii = MoaiPii.load();                       // MOAIPII_LIB → -Dmoaipii.lib → ./lib → ./ → 시스템
 * MoaiPii.Result r = pii.scan(text, Arrays.asList("RRN", "PHONE"));
 * for (MoaiPii.Item it : r.items) System.out.println(it.label + " " + text.substring(it.start, it.end));
 * }</pre>
 *
 * <p>좌표는 기본으로 UTF-16 단위다({@code offsets = "utf16"}) — Java {@code String.substring} 에 그대로 쓴다.
 * 규칙은 C 헤더 moaipii.h 와 같다. 여러 스레드에서 동시에 불러도 된다.
 * 라이브러리가 out 으로 준 문자열은 이 클래스가 읽은 즉시 moaipii_free 로 돌려준다(라이브러리가 0 으로 덮는다).
 */
public final class MoaiPii {

    /** 이 감싸개가 아는 ABI 판. 라이브러리의 moaipii_abi_version() 과 같아야 한다. */
    public static final int ABI_VERSION = 1;

    // 반환 코드 (moaipii.h 의 mp_status)
    public static final int MP_OK = 0;
    public static final int MP_ERR_ARG = 1;
    public static final int MP_ERR_UTF8 = 2;
    public static final int MP_ERR_PANIC = 99;

    /**
     * JNA 가 C 함수에 이어 붙이는 선언.
     * size_t 는 JNA 에 맞는 형이 없어 long 으로 받는다 — 배포 대상이 x64 두 개뿐이라 64비트다(load 에서 확인).
     */
    interface Lib extends Library {
        int moaipii_abi_version();
        Pointer moaipii_version();
        Pointer moaipii_kopii_basis();
        int moaipii_labels(PointerByReference outJson);
        int moaipii_scan(byte[] text, long len, byte[] optsJson, PointerByReference outJson);
        void moaipii_free(Pointer p);
    }

    /** 실패. kind = "arg"|"utf8"|"panic"|"load"|"abi". code 는 라이브러리 반환 코드(불러오기 실패면 -1). */
    public static class MoaiPiiException extends RuntimeException {
        public final int code;
        public final String kind;

        MoaiPiiException(int code, String kind, String message) {
            super(message);
            this.code = code;
            this.kind = kind;
        }
    }

    /** 검사 옵션. 기본값: 위치 있음 · 값 없음 · UTF-16 좌표 · 깨진 UTF-8 은 오류 · 모르는 라벨은 오류. */
    public static final class Options {
        public boolean items = true;
        public boolean values = false;
        /** "utf16"(Java 기본) | "char" | "byte" */
        public String offsets = "utf16";
        /** "error" | "replace" — scanBytes 에서만 뜻이 있다 */
        public String invalidUtf8 = "error";
        /** "error" | "ignore" */
        public String unknownLabels = "error";
        /** null(시스템 날짜) 또는 "YYYY-MM-DD" */
        public String today = null;
    }

    /** 검출 한 건. 좌표는 [start, end). value 는 Options.values 일 때만. */
    public static final class Item {
        public final String label;
        public final int start;
        public final int end;
        public final String value;

        Item(String label, int start, int end, String value) {
            this.label = label;
            this.start = start;
            this.end = end;
            this.value = value;
        }

        @Override
        public String toString() {
            return label + "[" + start + "," + end + ")";
        }
    }

    /** 검사 결과. */
    public static final class Result {
        /** 본문 코드포인트 수 */
        public final long chars;
        /** 라벨별 건수(0 건은 없다) */
        public final Map<String, Long> counts;
        /** 겹침 해소가 끝난 위치 목록(문서 순서) */
        public final List<Item> items;
        /** 무시한 모르는 라벨(unknownLabels = "ignore" 일 때) */
        public final List<String> unknown;
        /** 라이브러리가 준 원문 JSON */
        public final String json;

        Result(long chars, Map<String, Long> counts, List<Item> items, List<String> unknown, String json) {
            this.chars = chars;
            this.counts = counts;
            this.items = items;
            this.unknown = unknown;
            this.json = json;
        }
    }

    /** scanJson 의 결과 — 반환 코드와 원문 JSON. */
    public static final class Raw {
        public final int code;
        public final String json;

        Raw(int code, String json) {
            this.code = code;
            this.json = json;
        }
    }

    private final Lib lib;
    private final String path;

    private MoaiPii(Lib lib, String path) {
        this.lib = lib;
        this.path = path;
    }

    /** 기본 순서로 찾아 연다: MOAIPII_LIB 환경변수 → -Dmoaipii.lib → ./lib/ → ./ → 시스템 경로. */
    public static MoaiPii load() {
        return load(null);
    }

    /**
     * 라이브러리를 연다. path 를 주면 그 파일만 연다.
     *
     * @throws MoaiPiiException kind="load"(못 찾음·못 엶), kind="abi"(함수 모양 판이 다름)
     */
    public static MoaiPii load(String path) {
        // size_t 를 long 으로 받으므로 64비트 JVM 이어야 한다.
        if (Native.SIZE_T_SIZE != 8) {
            throw new MoaiPiiException(-1, "load", "64비트 JVM 이 필요합니다(size_t=" + Native.SIZE_T_SIZE + ")");
        }
        List<String> tried = new ArrayList<String>();
        for (String cand : candidates(path)) {
            tried.add(cand);
            if (cand.indexOf(File.separatorChar) >= 0 && !new File(cand).isFile()) continue;
            Lib l;
            try {
                l = Native.load(cand, Lib.class);
            } catch (UnsatisfiedLinkError e) {
                if (path != null) throw new MoaiPiiException(-1, "load", "라이브러리를 열지 못했습니다: " + cand + " (" + e.getMessage() + ")");
                continue;
            }
            int abi = l.moaipii_abi_version();
            if (abi != ABI_VERSION) {
                throw new MoaiPiiException(-1, "abi", "moaipii ABI 판이 다릅니다: 라이브러리 " + abi
                        + ", 감싸개 " + ABI_VERSION + " (" + cand + ")");
            }
            return new MoaiPii(l, cand);
        }
        throw new MoaiPiiException(-1, "load", "moaipii 라이브러리를 찾지 못했습니다. 찾아본 곳: " + tried
                + " (환경변수 MOAIPII_LIB 로 알려 주세요)");
    }

    // 찾을 후보 목록(앞에서부터)
    private static List<String> candidates(String explicit) {
        List<String> c = new ArrayList<String>();
        if (explicit != null) { c.add(explicit); return c; }
        String file = System.getProperty("os.name").toLowerCase().startsWith("win") ? "moaipii.dll" : "libmoaipii.so";
        String env = System.getenv("MOAIPII_LIB");
        if (env != null && !env.isEmpty()) c.add(new File(env).isDirectory() ? new File(env, file).getAbsolutePath() : env);
        String prop = System.getProperty("moaipii.lib");
        if (prop != null && !prop.isEmpty()) c.add(prop);
        c.add(new File("lib", file).getAbsolutePath());
        c.add(new File(file).getAbsolutePath());
        // 이름만 주면 JNA 가 jna.library.path · java.library.path · 시스템 경로에서 찾는다.
        c.add("moaipii");
        return c;
    }

    /** 연 라이브러리 경로(또는 이름) */
    public String libPath() { return path; }

    /** 라이브러리 버전 "0.1.0+abc1234" */
    public String version() { return utf8(lib.moaipii_version()); }

    /** 검출 기준이 된 ko-pii 판 "1.15.2" */
    public String kopiiBasis() { return utf8(lib.moaipii_kopii_basis()); }

    /** 지원 라벨 목록(별칭 제외) */
    @SuppressWarnings("unchecked")
    public List<String> labels() {
        PointerByReference out = new PointerByReference();
        int code = lib.moaipii_labels(out);
        Map<String, Object> m = result(code, take(out));
        List<String> r = new ArrayList<String>();
        for (Object o : (List<Object>) m.get("labels")) r.add((String) o);
        return r;
    }

    /** 본문 검사(기본 옵션: UTF-16 좌표). */
    public Result scan(String text, Collection<String> labels) {
        return scan(text, labels, new Options());
    }

    /**
     * 본문 검사. 짝 없는 서로게이트는 UTF-8 로 바꿀 때 "?" 한 글자가 되므로 UTF-16 좌표가 그대로 맞는다.
     *
     * @throws MoaiPiiException 인자·옵션 오류(arg), 내부 오류(panic)
     */
    public Result scan(String text, Collection<String> labels, Options o) {
        return scanBytes(text.getBytes(StandardCharsets.UTF_8), labels, o);
    }

    /**
     * UTF-8 바이트를 그대로 검사한다(깨진 바이트는 Options.invalidUtf8 에 따라).
     * 호출이 끝나면 넘긴 배열을 0 으로 지운다 — 개인정보 사본을 힙에 남기지 않으려는 것이다.
     */
    public Result scanBytes(byte[] utf8, Collection<String> labels, Options o) {
        Raw raw = scanJson(utf8, optsJson(labels, o));
        Map<String, Object> m = result(raw.code, raw.json);
        return toResult(m, raw.json);
    }

    /** 옵션 JSON 을 직접 넘기고 원문 JSON 을 받는다(실패도 예외 없이 코드로). 넘긴 배열은 0 으로 지운다. */
    public Raw scanJson(byte[] utf8, String optsJson) {
        PointerByReference out = new PointerByReference();
        // JNA 는 byte[] 를 호출 동안 네이티브 메모리로 복사해 넘긴다. 끝에 NUL 을 붙여 둔다(옵션은 C 문자열).
        byte[] ob = optsJson.getBytes(StandardCharsets.UTF_8);
        byte[] opts = Arrays.copyOf(ob, ob.length + 1);
        try {
            int code = lib.moaipii_scan(utf8, utf8.length, opts, out);
            return new Raw(code, take(out));
        } finally {
            Arrays.fill(utf8, (byte) 0);
        }
    }

    // 옵션 JSON 만들기(라벨 차례를 고정해 같은 입력이면 같은 JSON)
    private static String optsJson(Collection<String> labels, Options o) {
        StringBuilder b = new StringBuilder("{\"labels\":[");
        boolean first = true;
        for (String l : new TreeSet<String>(labels)) {
            if (!first) b.append(',');
            b.append(Json.quote(l));
            first = false;
        }
        b.append("],\"items\":").append(o.items)
         .append(",\"values\":").append(o.values)
         .append(",\"offsets\":").append(Json.quote(o.offsets))
         .append(",\"invalid_utf8\":").append(Json.quote(o.invalidUtf8))
         .append(",\"unknown_labels\":").append(Json.quote(o.unknownLabels))
         .append(",\"today\":").append(Json.quote(o.today))
         .append('}');
        return b.toString();
    }

    // out 으로 받은 문자열을 읽고 곧바로 돌려준다
    private String take(PointerByReference out) {
        Pointer p = out.getValue();
        if (p == null) return "";
        try {
            return p.getString(0, StandardCharsets.UTF_8.name());
        } finally {
            lib.moaipii_free(p);
        }
    }

    // 결과 JSON → Map, 실패 코드면 예외
    @SuppressWarnings("unchecked")
    private static Map<String, Object> result(int code, String json) {
        Map<String, Object> m;
        try {
            m = json.isEmpty() ? new LinkedHashMap<String, Object>() : (Map<String, Object>) Json.parse(json);
        } catch (RuntimeException e) {
            throw new MoaiPiiException(code, "panic", "결과 JSON 오류: " + e.getMessage());
        }
        if (code != MP_OK) {
            Map<String, Object> e = m.get("error") instanceof Map ? (Map<String, Object>) m.get("error") : new LinkedHashMap<String, Object>();
            String kind = e.get("kind") instanceof String ? (String) e.get("kind") : "panic";
            String msg = e.get("message") instanceof String ? (String) e.get("message") : "알 수 없는 오류";
            throw new MoaiPiiException(code, kind, msg);
        }
        return m;
    }

    // Map → Result
    @SuppressWarnings("unchecked")
    private static Result toResult(Map<String, Object> m, String json) {
        Map<String, Long> counts = new LinkedHashMap<String, Long>();
        Object c = m.get("counts");
        if (c instanceof Map) {
            for (Map.Entry<String, Object> e : ((Map<String, Object>) c).entrySet()) counts.put(e.getKey(), num(e.getValue()));
        }
        List<Item> items = new ArrayList<Item>();
        Object a = m.get("items");
        if (a instanceof List) {
            for (Object x : (List<Object>) a) {
                Map<String, Object> it = (Map<String, Object>) x;
                items.add(new Item((String) it.get("label"), (int) num(it.get("start")), (int) num(it.get("end")),
                        it.get("value") instanceof String ? (String) it.get("value") : null));
            }
        }
        List<String> unknown = new ArrayList<String>();
        Object u = m.get("unknown");
        if (u instanceof List) for (Object x : (List<Object>) u) unknown.add((String) x);
        return new Result(num(m.get("chars")), Collections.unmodifiableMap(counts),
                Collections.unmodifiableList(items), Collections.unmodifiableList(unknown), json);
    }

    private static String utf8(Pointer p) {
        return p == null ? null : p.getString(0, StandardCharsets.UTF_8.name());
    }

    private static long num(Object o) {
        return o instanceof Number ? ((Number) o).longValue() : 0L;
    }
}
