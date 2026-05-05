// 토스트 단일 진입점. 컴포넌트는 직접 sonner를 import하지 말고 여기서 가져간다.
//
// 정책 (v0.3.2 도입):
//   - success: 사용자 액션이 성공했고 페이지가 자동으로 안 바뀌는 경우(다이얼로그 안 저장 등)
//   - error  : 인라인으로 표시하기 마땅찮은 비차단 오류(백그라운드 인덱싱 실패 등).
//              모달 안 즉시 차단 오류는 setError로 인라인 유지.
//   - info   : 백그라운드 사건의 가벼운 알림(인덱싱 시작 등). 과도하면 끄자.
//
// 단일 진입점이라 향후 라이브러리 교체나 테스트 mock도 이 파일만 건드리면 된다.

import { toast as sonnerToast } from "sonner";

export const toast = {
  success: (message: string) => sonnerToast.success(message),
  error: (message: string) => sonnerToast.error(message),
  info: (message: string) => sonnerToast.info(message),
  message: (message: string) => sonnerToast(message),
};
