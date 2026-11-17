<?php
/**
 * 감싸개 공통 시험 — PHP 실행기.
 * cases.json 의 사례를 PHP 감싸개(bindings/php/MoaiPii.php)로 돌려 한 줄에 한 사례씩 JSON 을 낸다.
 *  1) 본문은 바이트 그대로(PHP 문자열)
 *  2) values 가 있고 좌표가 이 언어의 단위(byte)면 substr 가 값과 같은지(slice_ok)
 *  3) fake_abi 는 MOAIPII_FAKE_LIB 를 불러 abi 오류가 나는지
 *  (PHP 는 기본으로 스레드가 없어 threads 사례는 돌리지 않는다)
 *
 * 사용: php -d ffi.enable=1 tests/bindings/php/runner.php <cases.json> <bindings/php 폴더>
 */
require $argv[2] . DIRECTORY_SEPARATOR . 'MoaiPii.php';

const LANG = 'php';

$doc = json_decode(file_get_contents($argv[1]), true);
$pii = MoaiPii::load();
foreach ($doc['cases'] as $c) {
    if (!in_array(LANG, $c['langs'], true)) continue;
    $out = ['name' => $c['name'], 'lang' => LANG];
    if (!empty($c['fake_abi'])) {
        try {
            MoaiPii::load(getenv('MOAIPII_FAKE_LIB'));
            $out['code'] = 0;
        } catch (MoaiPiiException $e) {
            $out['code'] = -1;
            $out['kind'] = $e->kind;
        }
        echo json_encode($out, JSON_UNESCAPED_UNICODE), "\n";
        continue;
    }
    $text = str_repeat(base64_decode($c['text_b64']), $c['repeat'] ?? 1);
    $o = $c['opts'];
    try {
        $r = $pii->scan($text, $c['labels'], $o);
        $out['code'] = 0;
        // 빈 객체가 빈 배열로 나가지 않게 객체로 둔다
        $out['counts'] = (object)$r['counts'];
        $out['items'] = $r['items'];
        $out['unknown'] = $r['unknown'] ?? [];
        // 이 언어의 자르기 단위(byte)면 원래 문자열을 잘라 값과 맞춰 본다
        if ($o['values'] && $o['offsets'] === 'byte') {
            $ok = true;
            foreach ($r['items'] as $it) {
                $ok = $ok && substr($text, $it['start'], $it['end'] - $it['start']) === $it['value'];
            }
            $out['slice_ok'] = $ok;
        }
    } catch (MoaiPiiException $e) {
        $out['code'] = $e->getCode();
        $out['kind'] = $e->kind;
    }
    echo json_encode($out, JSON_UNESCAPED_UNICODE), "\n";
}
