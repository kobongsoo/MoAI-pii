<?php
/**
 * moaipii.dll / libmoaipii.so 의 PHP 감싸개 (PHP 7.4 이상, FFI 확장).
 *
 * 정본: MoAI-pii/bindings/php/MoaiPii.php — 쓰는 프로젝트는 이 파일을 복사해 둔다.
 *
 * 준비
 *   - php.ini 에 extension=ffi (Windows 는 php_ffi.dll 이 ext/ 에 있다)
 *   - CLI 는 기본 설정(ffi.enable=preload)으로 바로 된다.
 *     웹(php-fpm·Apache 모듈)에서 쓰려면 ffi.enable=true 로 두거나 opcache.preload 로 미리 불러야 한다.
 *
 * 사용
 *   require 'MoaiPii.php';
 *   $pii = MoaiPii::load();                          // MOAIPII_LIB → ./lib/ → ./ → 이 파일 옆 → 시스템
 *   $r   = $pii->scan($text, ['RRN', 'PHONE']);      // ['chars'=>…, 'counts'=>[…], 'items'=>[…]]
 *   foreach (array_reverse($r['items']) as $it)      // 좌표는 바이트 — substr 에 그대로 쓴다
 *       $text = substr_replace($text, '***', $it['start'], $it['end'] - $it['start']);
 *
 * 좌표는 기본으로 바이트 단위(offsets = "byte"), 깨진 UTF-8 은 기본으로 U+FFFD 로 보고 검사한다
 * (invalid_utf8 = "replace"). 그래도 byte 좌표는 넘긴 원래 문자열을 가리킨다.
 * EUC-KR 같은 다른 인코딩 본문은 먼저 mb_convert_encoding($s, 'UTF-8', 'EUC-KR') 로 바꿔서 넘긴다.
 *
 * 규칙은 C 헤더 moaipii.h 와 같다. out 으로 받은 문자열은 이 클래스가 읽은 즉시 moaipii_free 로 돌려준다.
 */

/** 실패. $kind = arg|utf8|panic|load|abi */
class MoaiPiiException extends RuntimeException
{
    /** @var string */
    public $kind;

    public function __construct(int $code, string $kind, string $message)
    {
        parent::__construct($message, $code);
        $this->kind = $kind;
    }
}

final class MoaiPii
{
    /** 이 감싸개가 아는 ABI 판. 라이브러리의 moaipii_abi_version() 과 같아야 한다. */
    const ABI_VERSION = 1;

    const MP_OK = 0;
    const MP_ERR_ARG = 1;
    const MP_ERR_UTF8 = 2;
    const MP_ERR_PANIC = 99;

    // FFI::cdef 는 전처리기(#include·#define·#ifdef)를 못 읽으므로 moaipii.h 의 선언만 옮겨 둔다.
    // 본문은 const char* 로 받아 PHP 문자열 버퍼를 그대로 넘긴다(길이는 따로 주므로 \0 이 있어도 된다).
    // (uint32_t·size_t 는 PHP FFI 가 미리 아는 형이라 typedef 하지 않는다.)
    const CDEF = <<<'C'
uint32_t    moaipii_abi_version(void);
const char* moaipii_version(void);
const char* moaipii_kopii_basis(void);
int         moaipii_labels(char** out_json);
int         moaipii_scan(const char* text, size_t len, const char* opts_json, char** out_json);
void        moaipii_free(char* p);
C;

    /** @var FFI */
    private $ffi;
    /** @var string */
    private $path;

    private function __construct(FFI $ffi, string $path)
    {
        $this->ffi = $ffi;
        $this->path = $path;
    }

    /**
     * 라이브러리를 연다. $path 를 주면 그 파일만 연다.
     * 기본 순서: MOAIPII_LIB 환경변수 → ./lib/ → ./ → 이 파일 폴더(와 그 lib/) → 이름만(시스템 경로).
     *
     * @throws MoaiPiiException kind=load(못 찾음·FFI 꺼짐), kind=abi(함수 모양 판이 다름)
     */
    public static function load(?string $path = null): self
    {
        if (!extension_loaded('ffi')) {
            throw new MoaiPiiException(-1, 'load', 'PHP FFI 확장이 꺼져 있습니다 (php.ini 에 extension=ffi, 웹이면 ffi.enable=true)');
        }
        if (PHP_INT_SIZE !== 8) {
            throw new MoaiPiiException(-1, 'load', '64비트 PHP 가 필요합니다');
        }
        $file = PHP_OS_FAMILY === 'Windows' ? 'moaipii.dll' : 'libmoaipii.so';
        if ($path !== null) {
            $cands = [$path];
        } else {
            $cands = [];
            $env = getenv('MOAIPII_LIB');
            if ($env) $cands[] = is_dir($env) ? $env . DIRECTORY_SEPARATOR . $file : $env;
            $cands[] = getcwd() . DIRECTORY_SEPARATOR . 'lib' . DIRECTORY_SEPARATOR . $file;
            $cands[] = getcwd() . DIRECTORY_SEPARATOR . $file;
            $cands[] = __DIR__ . DIRECTORY_SEPARATOR . $file;
            $cands[] = __DIR__ . DIRECTORY_SEPARATOR . 'lib' . DIRECTORY_SEPARATOR . $file;
            $cands[] = dirname(__DIR__, 2) . DIRECTORY_SEPARATOR . 'lib' . DIRECTORY_SEPARATOR . $file;
            $cands[] = $file;
        }
        $errors = [];
        foreach ($cands as $c) {
            // 경로가 들어 있는데 파일이 없으면 열어 보지도 않는다.
            if ((strpbrk($c, '/\\') !== false) && !is_file($c)) { $errors[] = $c; continue; }
            try {
                $ffi = FFI::cdef(self::CDEF, $c);
            } catch (FFI\Exception $e) {
                $errors[] = $c . ' (' . $e->getMessage() . ')';
                continue;
            }
            $abi = $ffi->moaipii_abi_version();
            if ($abi !== self::ABI_VERSION) {
                throw new MoaiPiiException(-1, 'abi', "moaipii ABI 판이 다릅니다: 라이브러리 $abi, 감싸개 " . self::ABI_VERSION . " ($c)");
            }
            return new self($ffi, $c);
        }
        throw new MoaiPiiException(-1, 'load', 'moaipii 라이브러리를 찾지 못했습니다. 찾아본 곳: ' . implode(', ', $errors)
            . ' (환경변수 MOAIPII_LIB 로 알려 주세요)');
    }

    /** 연 라이브러리 경로 */
    public function libPath(): string { return $this->path; }

    /** 라이브러리 버전 "0.1.0+abc1234" (const char* 반환은 FFI 가 PHP 문자열로 바꿔 준다) */
    public function version(): string { return $this->ffi->moaipii_version(); }

    /** 검출 기준이 된 ko-pii 판 "1.15.2" */
    public function kopiiBasis(): string { return $this->ffi->moaipii_kopii_basis(); }

    /** 지원 라벨 목록(별칭 제외) */
    public function labels(): array
    {
        $out = $this->ffi->new('char*');
        $code = $this->ffi->moaipii_labels(FFI::addr($out));
        return $this->checked($code, $this->take($out))['labels'];
    }

    /**
     * 본문 검사.
     *
     * @param string $text   본문(UTF-8 바이트열)
     * @param array  $labels 켤 라벨들
     * @param array  $opts   items·values·offsets(기본 "byte")·invalid_utf8(기본 "replace")·unknown_labels·today
     * @return array ['chars'=>int, 'counts'=>[라벨=>건수], 'items'=>[['label','start','end'(,'value')]], ('unknown'=>[…])]
     * @throws MoaiPiiException 인자·옵션 오류(arg), 본문 UTF-8 아님(utf8 — invalid_utf8="error" 일 때), 내부 오류(panic)
     */
    public function scan(string $text, array $labels, array $opts = []): array
    {
        $o = $opts + ['items' => true, 'values' => false, 'offsets' => 'byte', 'invalid_utf8' => 'replace',
                      'unknown_labels' => 'error', 'today' => null];
        $ls = array_values(array_unique($labels));
        sort($ls);
        $o['labels'] = $ls;
        [$code, $json] = $this->scanJson($text, json_encode($o, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES));
        $r = $this->checked($code, $json);
        // json_decode 는 빈 객체를 빈 배열로 준다 — counts 는 늘 배열이다.
        $r['counts'] = $r['counts'] ?? [];
        return $r;
    }

    /**
     * 옵션 JSON 을 직접 넘기고 원문 JSON 을 받는다(실패도 예외 없이 코드로).
     * @return array [반환 코드, 결과 JSON 문자열]
     */
    public function scanJson(string $text, string $optsJson): array
    {
        $out = $this->ffi->new('char*');
        $code = $this->ffi->moaipii_scan($text, strlen($text), $optsJson, FFI::addr($out));
        return [$code, $this->take($out)];
    }

    // out 으로 받은 문자열을 읽고 바로 해제한다(성공이든 실패든 해제해야 한다).
    private function take($out): string
    {
        if (FFI::isNull($out)) {
            throw new MoaiPiiException(self::MP_ERR_PANIC, 'panic', 'moaipii 가 결과를 주지 않았습니다');
        }
        try {
            return FFI::string($out);
        } finally {
            $this->ffi->moaipii_free($out);
        }
    }

    // 코드가 0 이 아니면 결과 JSON 의 error 로 예외를 만든다.
    private function checked(int $code, string $json): array
    {
        $v = json_decode($json, true);
        if ($code === self::MP_OK) {
            if (!is_array($v)) throw new MoaiPiiException(self::MP_ERR_PANIC, 'panic', "결과 JSON 을 읽지 못했습니다");
            return $v;
        }
        $e = (is_array($v) && isset($v['error']) && is_array($v['error'])) ? $v['error'] : [];
        throw new MoaiPiiException($code, (string)($e['kind'] ?? 'panic'), (string)($e['message'] ?? '알 수 없는 오류'));
    }
}
