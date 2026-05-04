// 사용자 입력 이름의 OS 금지문자 즉시 strip 유틸 (옵시디언 패턴).
//
// 백엔드(`commands::study::sanitize_to_slug`)와 동일 정책:
//   * 금지문자 9개: / \ : * ? " < > |
//   * Control char (0x00 ~ 0x1F)
// 시작/끝 공백·점 거부, 길이 제한, Windows 예약어 처리는 백엔드에서.
// 프론트엔드는 *입력 시점에* 금지문자가 화면에 박히지 않게만 막아 준다.

// eslint-disable-next-line no-control-regex
const FORBIDDEN_PATTERN = /[/\\:*?"<>|\x00-\x1F]/g;

/** 금지문자를 즉시 제거. onChange 핸들러에서 호출. */
export function stripForbiddenChars(input: string): string {
  return input.replace(FORBIDDEN_PATTERN, "");
}

/** 입력 문자열이 금지문자를 포함하는지 검사 — 안내 표시용.
 *  RegExp.prototype.test는 g flag일 때 stateful이라 매 호출마다 lastIndex 리셋. */
export function hasForbiddenChars(input: string): boolean {
  FORBIDDEN_PATTERN.lastIndex = 0;
  return FORBIDDEN_PATTERN.test(input);
}
