# Changelog

본 레포의 변경 사항은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/) 형식을 따른다.
버전 번호는 [Semantic Versioning](https://semver.org/lang/ko/)을 따른다.

## [Unreleased]

### Fixed (PR 56 — v0.3.1: 스터디 시작하기 UNIQUE constraint 위반)
- 사용자 보고 — "스터디 시작하기" 클릭 시 `UNIQUE constraint failed: studies.is_active`
- 원인: `activate` 함수가 단일 `UPDATE ... CASE WHEN`으로 모든 row를 한 번에 갱신. SQLite는 partial unique index(`WHERE is_active = 1`)에 deferred 지원 X → row 단위 처리 중 *대상 row가 1로 바뀌는 시점*에 *기존 active row도 아직 1*이라 즉시 위반
- 픽스: 두 단계 UPDATE — (1) 모든 active row를 0으로 (2) 대상 slug만 1로. 같은 트랜잭션 안
- 회귀 방지 테스트 추가 — `activate_with_existing_active_does_not_violate_unique`

### Changed (PR 55 — v0.3.1: 시작하기 진행 안 됨 진단)
- Library.handleEnter에 try-catch + `enteringSlug`/`enterError` state. select 실패 시 console.error + Inspector footer에 inline 에러 표시
- LibraryInspector에 `entering`/`enterError` prop 추가, 진입 버튼 disabled + Loader2 spinner

### Changed (PR 54 — v0.3.1: 인스펙터 버튼 라벨 상황별 분기)
- 사용자 명시 — 활성 스터디는 "이어하기", 비활성은 "스터디 시작하기"로 라벨 분기
- LibraryInspector의 진입 버튼이 `study.is_active`에 따라 `library.inspector.continue` / `library.inspector.start`
- ko.json에 두 키 추가. 기존 `enter` 키는 보존(향후 다른 진입 경로 있을 때 재사용)

### Changed (PR 53 — v0.3.1: 라이브러리 활성 표시를 책 펼친 아이콘으로)
- 사용자 명시 — "활성" 단어가 일반 사용자에게 안 와닿음. 책 펼친 아이콘(BookOpen)으로 *열려 있음* 표현
- Library 카드 cover: 텍스트 "활성" → 검은 원형 뱃지 안에 BookOpen 아이콘. aria-label/title로 의미 보존
- LibraryInspector 헤더: 텍스트 "활성" 뱃지 → BookOpen 아이콘 + "열려 있음" 텍스트
- ko.json `library.active_badge` "활성" → "열려 있음"

### Changed (PR 52 — v0.3.1: 첫 토글 추가는 active group에 탭 추가)
- 사용자 명시 — 기존에 default로 추가한 적 없는 패널을 토글할 때 새 그룹 대신 *마지막으로 조작한 그룹(active group)*에 탭으로 추가
- `resolveAddPosition` 우선순위 갱신:
  1. `groupMemory`의 살아있는 group ID → within (PR 44)
  2. `api.activeGroup?.id` → within (NEW, PR 52)
  3. `DEFAULT_POSITIONS` → fallback (활성 그룹조차 없는 첫 진입 케이스)

### Changed (PR 51 — v0.3.1: TOC 사이드바 내부 헤더 제거)
- 사용자 명시 — TOC 패널 안의 "TOC" 헤더 + 활성 스터디 메타 영역 제거. dockview 탭(아이콘+제목)과 TopBar 활성 스터디 칩에 같은 정보 이미 있음
- StudySidebar에서 PaneHeader/PaneTitle/active_study 박스 삭제. PaneBody만 유지
- `onClose` prop과 `BookOpen`/`PaneHeader`/`PaneTitle` import도 unused로 제거

### Changed (PR 50 — v0.3.1: dockview 탭에 아이콘 추가)
- 사용자 명시 — 토글 탭(dockview 패널 탭 헤더)에 아이콘 같이 표시
- `src/lib/panelIcons.tsx` 신규 — 9 패널 lucide 아이콘 매핑(`DockPanelId → LucideIcon`). TopBar 토글과 dockview 탭이 공유
- `src/components/dockview/PanelTab.tsx` 신규 — 커스텀 탭 헤더 컴포넌트. 아이콘 + 제목(`api.title`, `onDidTitleChange` 구독) + close 버튼
- `<DockviewReact defaultTabComponent={PanelTab} />` 박힘
- TopBar 토글이 새 `PANEL_ICONS` 매핑 재사용 — 인라인 아이콘 정의 제거

### Fixed (PR 49 — v0.3.1: PDF 깨짐 보강 — 강제 재렌더)
- 사용자 보고 — PR 48 `defaultRenderer='always'`에도 PDF 여전히 흰 캔버스
- 추가 분석: 'always' 모드의 OverlayRenderContainer가 패널을 absolute로 띄우더라도, 브라우저(특히 WebKit/WebView2)별로 *DOM ownership 변경 시 canvas GPU 컨텍스트 보존이 보장되지 않음*. fromJSON({reuseExistingPanels:true}) 흐름의 `moveGroupOrPanel`(임시 group → 원래 위치) 단계에서 canvas 콘텐츠 손실
- 픽스: `airis:pdf-rerender` CustomEvent 신호 추가
  - Workspace의 `tryRestoreSnapshot` 후 `requestAnimationFrame` 안에서 dispatch
  - `BookViewer.PdfContent`가 listen → `rerenderTick` state 증가 → 페이지 render `useEffect` 강제 재실행
- canvas가 흰 상태였어도 즉시 다시 그려짐

### Fixed (PR 48 — v0.3.1: PDF 렌더링 깨짐 버그)
- 사용자 보고 — 노트 탭을 단독 그룹에 둔 후 토글 비활성화→활성화 시 PDF 렌더링 깨짐
- 원인: dockview default renderer = `onlyWhenVisible` → 패널 reorganization 중 DOM detach. BookViewer의 pdfjs canvas는 detach 시 GPU 콘텐츠 손실, 재attach 시 빈 캔버스. snapshot fromJSON({reuseExistingPanels:true}) 흐름의 *임시 group → 원래 위치* 이동 단계에서 발생
- 픽스: `<DockviewReact defaultRenderer="always" />` — 패널이 DOM에 항상 유지(absolute positioning), detach/attach 없음. 메모리 약간 늘지만 데스크톱 앱이라 무관

### Changed (PR 47 — v0.3.1: TopBar 9 토글 + TOC/Viewer/Chat 복구 수단)
- 사용자 보고 — 뷰어/TOC/챗 패널을 닫으면 단축키 외 복구 수단 없음 (뷰어는 단축키도 없음)
- TopBar에 CORE 토글 (TOC / Viewer / Chat) 추가. 기존 SLIDEUP 6 토글과 *시각 구분선* 분리
- 9 토글 + Settings 구조: `[Brand] [Library] > [Workspace] | spacer | [TOC][Viewer][Chat] | [Quiz][Notes][SRS][Progress][Memory][Pomodoro] | [Settings]`
- `DockPanelId`에 `toc | viewer | chat` 추가 (Workspace의 `PanelId`와 정렬)
- ko.json `topbar.toggle_toc/viewer/chat` 키 추가

### Performance (PR 46 — v0.3.1: 그룹 복원 속도 개선)
- 사용자 보고 — 단독 group 폐기 후 패널 재오픈 시 *복원이 너무 느림*
- 원인 1: `api.fromJSON(snapshot)` default가 `reuseExistingPanels: false` → 7개 패널 모두 unmount + remount + 비싼 mount effect 동시 폭발 (BookViewer markdown 파싱, ChatPanel hydrate, IPC 호출 7+개 큐잉 등)
- 원인 2: fromJSON 내부에서 layout change 이벤트가 5~10번 폭발 → onDidLayoutChange가 매번 toJSON+JSON.stringify+localStorage.setItem 동기 IO
- 픽스 1: `api.fromJSON(snapshot, { reuseExistingPanels: true })` — 살아있는 패널은 임시 group으로 옮긴 후 재배치. unmount/remount 없음
- 픽스 2: layout save에 200ms debounce — fromJSON storm 동안의 IO 누적 차단

### Changed (PR 45 — v0.3.1: 단독 group 폐기 시 layout snapshot 복원)
- 사용자 명시 — group에 단독으로 있는 panel을 닫으면 group 자체가 폐기되어 다시 열 때 default 위치로 떨어지는 문제
- 두 단계 메모리 — group이 살아있는 케이스(다른 panel과 함께)는 group ID, group이 폐기되는 케이스(단독)는 전체 layout snapshot
- close 직전 `group.panels.length` 확인:
  - `> 1` → group ID 저장 (PR 44 그대로)
  - `=== 1` → `api.toJSON()`으로 layout snapshot 저장
- add 시 우선순위: 살아있는 group ID → snapshot fromJSON → DEFAULT_POSITIONS
- 부작용: snapshot fromJSON은 *다른 패널의 그 사이 변경*도 함께 snapshot 시점으로 되돌림 (close → 다른 패널 옮김 → 재오픈 시 옮긴 변경 사라짐). v0.3.1 후속에서 grid tree 분석으로 정교화 가능

### Changed (PR 44 — v0.3.1: TopBar 토글 아이콘만 + 패널 위치 메모리)
- 사용자 명시 — TopBar 6 토글 라벨 제거 (아이콘만, hover tooltip)
- 사용자 명시 — 패널 on/off 시 직전 위치(group) 복원
- TopBar 토글 = `h-8 w-8` 정사각 버튼, 라벨 span 제거. `title` attribute로 hover tooltip
- `lastPositionRef = useRef<Map<PanelId, string>>` — close 직전 panel.api.group.id 저장
- `resolveAddPosition` — memory에 살아있는 group ID가 있으면 `referenceGroup + within`. 없으면 DEFAULT_POSITIONS fallback
- `togglePanel`/`focusOrAddPanel` 시그니처에 memory 인자 추가. close → save, add → use+clear

### Changed (PR 43 — v0.3.1: TopBar 우측 컨트롤 재구성 + Pomodoro 패널화)
- 사용자 명시 — 토글 탭(활성/비활성) 직관 + 라이트·다크/언어/오프라인은 Settings로 흡수 + Pomodoro 토글 탭 독립
- TopBar 우측 = **6 토글 + Settings** (Quiz / Notes / SRS / Progress / Memory / Pomodoro / | / Settings)
- `PomodoroInline.tsx` 삭제 → `PomodoroPanelContent.tsx` 신설 (시작·정지 + 인터럽션 사유 입력 + mm:ss 카운터)
- Workspace에 pomodoro 패널 추가, 6탭 그룹화 (Quiz/Notes/SRS/Progress/Memory/Pomodoro)
- 오프라인 토글 제거 — `uiStore.offline` 상태 + `OFFLINE_KEY` localStorage + ko.json `topbar.offline_*` 모두 폐기
- 단축키 도움말 버튼·Wifi·KO 라벨·테마 토글 모두 TopBar에서 제거. Settings 모달 안 섹션으로 흡수 (이미 있음)
- TopBar 토글 클릭 = `uiStore.requestPanelToggle(id)` → Workspace effect가 dockview API 호출. Library/Welcome에서 클릭하면 자동으로 워크스페이스로 이동 + 토글
- Slideup 잔존물 정리 — `SlideupTabs.tsx`, `SlideupPanel.tsx` 삭제. `uiStore.slideupTab` 상태 제거 (PR 42 dockview 도입 후 dead)
- ko.json `topbar.toggle_*` 키 추가, `topbar.offline_*`/`lang_pending` 제거. `pomodoro.interruption_placeholder` 추가
- decision-log D-072 추가

### Added (PR 42 — v0.3.1: dockview 도커블 워크스페이스)
- v0.3.1 첫 단계 — 워크스페이스를 `dockview-react` 6.0 기반 도커블 셸로 재구성
- 패널 8종 (toc / viewer / chat / quiz / notes / srs / progress / memory) 모두 dockview로 관리. 드래그 재배치 + splitter 리사이즈 + 같은 zone 묶이면 탭화
- 분리 차단 — `disableFloatingGroups`로 popout window 봉인 (사용자 명시: 분리 불가)
- **각 스터디별 레이아웃 persist** — `airis.layout.<study_slug>` localStorage에 `api.toJSON()` 저장. 활성 스터디 전환 시 layout reload
- 기본 레이아웃 — 좌측 TOC(260) / 중앙 viewer / 우측 chat(380) / 하단 5탭(quiz·notes·srs·progress·memory) 그룹 (initialHeight 280)
- 단축키 — `Mod+B`(toc 토글), `Mod+J`(chat 토글), `Mod+1~5`(slideup 패널 활성/추가). 닫혔다 다시 열면 default 위치 복귀
- `Mod+L` 챗 입력 포커스 — dockview 안에서 ref 직접 접근 어려워 `airis:focus-chat-input` CustomEvent로 위임
- dockview 테마 매핑 — `src/styles/dockview-theme.css`에 `.dockview-theme-airis` 클래스로 우리 oklch 토큰을 dockview CSS 변수에 매핑 (활성 탭 primary underline 등 prototype 정렬)
- App.tsx 단축키 정리 — 워크스페이스 단축키는 Workspace 컴포넌트로 이동. 글로벌 단축키만 App.tsx에 잔존
- ko.json `workspace.panel_*` 키 추가
- 새 의존: `dockview-react@6.0.0` (~200KB, 데스크톱 앱이라 통신 비용 무관)

### Changed (PR 41 — v0.3 보강 5: 인스펙터 너비 확대)
- 사용자 명시 — 라이브러리 인스펙터 너비 360 → 480px (유니티 인스펙터 표준 폭)
- 메인 영역 padding 376 → 496px

### Changed (PR 40 — v0.3 보강 4: 라이브러리 우측 인스펙터)
- 사용자 제안 — 라이브러리 카드 클릭 흐름 변경: 즉시 진입 → 우측 인스펙터(360px floating) 슬라이드 인
- 인스펙터 콘텐츠 — cover 미니어처 + 이름·슬러그·활성 뱃지 + 메타(책 수/마지막 사용/생성일) + 등록된 책 list (주/부 분리) + 진입/삭제 액션
- 카드 클릭 = `setInspectorSlug(slug)` (활성 전환 X). 다른 카드 클릭 시 인스펙터 콘텐츠 교체. ESC/X로 닫기
- 카드 hover 삭제 버튼 제거 — 인스펙터 안으로 흡수
- 메인 영역에 인스펙터 너비만큼 padding 자동 push (transition-padding)
- "진입" 버튼만 활성 전환 + workspace 이동
- `slideInRight` keyframes 추가
- ko.json `library.inspector.*` 키 추가

### Changed (PR 39 — v0.3 보강 3: 마법사 주교재/부교재 step 통합)
- 새 스터디 마법사 5-step → 4-step (이름 → Overview → 교재 → 인덱싱)
- Step 3 "교재" = 주교재(필수 1권) + 부교재(선택 N권)을 한 화면에 위/아래 섹션으로 표시
- ko.json `new_study.step3_label` "주교재" → "교재", `step4_label` "부교재" → "인덱싱", `step5_label` 제거
- `main_label`, `sub_label` 키 추가 (섹션 헤더)

### Changed (PR 38 — v0.3 보강 2: Settings LLM 그룹 통합 + 인증 흐름 일체)
- LLM 그룹: API 키/모델/예산 분리에서 *모델 선택*(통합) + *토큰 예산*(별도, placeholder)으로 단순화
- "모델 선택" 섹션 = 프로바이더 라디오 카드 3개. 활성만 펼쳐 안에 모델 + 인증 방식 + 인증 영역 + 연결 테스트 표시
- 인증 영역 조건부 — CLI 선택 시 CliPanel(설치/로그인 상태 + CLI 연결 다이얼로그 + 다시 확인 버튼). API 키 선택 시 ApiKeyInput
- Race condition 방지 — 프로바이더/인증 방식 전환 시 `await update` 끝날 때까지 다른 라디오 잠금 (Loader2 spinner). 활성된 것은 그대로 두고 비활성만 잠금
- "연결 테스트" 버튼 — `cli_status` + provider별 `cli_auth_status_*` 호출. 자동으로 진입 시 한 번 실행
- ko.json `settings.nav.llm_models`, `settings.llm.*` 키 추가. `nav.llm_key`/`nav.llm_model` 제거

### Changed (PR 37 — v0.3 보강: Settings 모달 prototype 정렬 + accent hue)
- Settings 모달 전면 재구성 — prototype `SettingsScreen`과 1:1: 좌측 nav(200px, 4 그룹) + 우측 콘텐츠 패널
- 그룹 4개 — LLM(API 키, 모델, 예산) · 학습 강도(메타인지, Memory, 검증) · UI·접근성(테마·언어, 접근성, 단축키) · 진단(사용량·비용)
- 인증 흐름 v0.2.1 D-066 보존 — llm-key 섹션이 *CLI 브릿지 카드* 메인 + *API 키 입력*(Advanced) 둘 다. prototype은 API 키만 보여주지만 우리는 둘 다 노출
- 테마 섹션 — 라이트/다크 토글 + density(컴팩트/보통/여유) + **accent hue 슬라이더(0~360°) + 5개 프리셋**
- ui-keys 클릭 시 ShortcutsDialog 열기로 위임
- 미구현 섹션은 placeholder 콘텐츠
- `uiStore.accentHue` 추가 (localStorage persist) → `<html style="--accent-h">` hookup
- 자체 RadioCard 컴포넌트 (prototype `.radio` 디자인)
- ko.json `settings.nav.*`, `settings.theme.*`, `settings.density.*`, `settings.accent.*`, `settings.placeholder` 키

### Changed (PR 36 — v0.3 마무리: Settings/Shortcuts 모달화 + dead code 정리)
- `Settings.tsx` 페이지 → 모달. backdrop / X 버튼 / Esc로 닫기. `Page` 타입에서 `"settings"` 제거
- `ShortcutsDialog.tsx` 신규 — `Mod+/`로 토글. prototype과 동일한 단축키 13개 list
- TopBar 설정 아이콘이 `setSettingsOpen(true)` 호출. ChatPanel/Welcome도 같은 store 사용
- dead code 삭제: `BookList.tsx`, `MemoryEditor.tsx`, `PomodoroPanel.tsx`
- uiStore에서 `memoryOpen`/`pomodoroOpen` 제거 (SlideupPanel/PomodoroInline으로 흡수됨)
- ko.json `shortcuts.*` 키 추가

### v0.3 슬라이스 종결 (PR 29~36)

D-067 (UX 정비), D-068 (합니다체), D-069 (마법사 + 슬러그 한국어), D-070 (prototype 100% 충실)이 박힌 6 트랙 슬라이스. 사용자 검증 후 v0.3.1 carryover로 학습 목표·마감일 사후 GUI 등 잔여 항목 진행.

### Changed (PR 35 — v0.3 트랙 D 3단계: 라이브러리 카드 + 마법사 5-step)
- `Library.tsx` 카드 디자인 — cover gradient(슬러그 hash로 hue 도출) + 큰 라벨(이름 첫 글자) + 진도 바 placeholder + 활성 뱃지. 헤더에 검색(disabled) + 새 스터디 primary
- `NewStudyDialog.tsx` 신규 — prototype 5-step 모달: 이름/언어 → Overview.md textarea → 주교재 → 부교재 → 인덱싱 안내. 트랜잭션 호출 + 백그라운드 인덱싱
- `Page` 타입에서 `"new-study"` 제거. `newStudyOpen` uiStore 토글로 전환
- `pages/NewStudyWizard.tsx` 삭제 — 페이지형 마법사 폐기
- ko.json `new_study.*` 키 신규 (5-step 라벨, summary, progress 메시지). `library.search`, `library.subtitle_count` 추가
- 진도 데이터(passed/total/streak)는 backend 미존재 — placeholder. v0.4 이후 hookup

### Changed (PR 34 — v0.3 트랙 D 2단계: SRS·Recall slideup + Pomodoro TopBar 인라인)
- `slideup/SrsDeckContent.tsx` 신규 — stat 4개(due 정확, 나머지 placeholder) + 대기 카드 list + "복습 시작" 버튼 → 기존 SrsPanel modal
- `slideup/QuizContent.tsx` 신규 — 회상 챌린지 안내 + "챌린지 시작" 버튼 → 기존 RecallPanel modal
- `PomodoroInline.tsx` 신규 — TopBar 인라인 카운터. 1초 polling, idle/running 시각, 클릭 토글
- `PomodoroPanel` 모달 trigger 제거 (App.tsx) + 단축키 `Mod+Shift+P` 제거
- 단축키 `Mod+K`(SRS), `Mod+R`(Recall) 제거 — modal은 slideup 시작 버튼으로만 트리거
- ko.json `srs.deck_*`/`stat_*`/`queued`, `recall.start_button` 키 추가

### Changed (PR 33 — v0.3 트랙 D 1단계: SlideupTabs + Memory 흡수)
- `src/components/layout/SlideupTabs.tsx` — 5탭(Quiz/Notes/SRS Deck/Progress/Memory). 활성 시 primary underline + soft 배경
- `src/components/layout/SlideupPanel.tsx` — bottom-sheet, 320px, BookViewer 영역 안의 absolute. SlideupTabs(36px) 위에 깔림
- `MemoryPanelContent` 신규 — 기존 MemoryEditor의 모달 wrapper 제거하고 콘텐츠만 추출. SlideupPanel의 Memory 탭으로 표시
- Workspace 중앙 영역에 SlideupTabs/SlideupPanel 박힘. relative container
- 단축키 `Mod+1`~`Mod+5`로 5 슬라이드업 탭 토글 (Quiz/Notes/SRS/Progress/Memory)
- App.tsx에서 MemoryEditor 모달 trigger 제거 (memoryOpen은 store에 잔존, PR 34에서 srs/recall과 함께 정리)
- `slideupTab` uiStore 추가
- tokens.css에 `@keyframes slideUp/fadeIn` 추가
- Notes/Progress/Quiz 탭은 placeholder. SRS·Recall hookup은 PR 34에서

### Changed (PR 32 — v0.3 트랙 C 2단계: 3-pane 셸 + StudySidebar TOC)
- `src/components/layout/Pane.tsx` 신규 — `Pane`/`PaneHeader`/`PaneTitle`/`PaneBody` 추상 (prototype `.pane` CSS 1:1)
- `src/components/StudySidebar.tsx` 신규 — 좌측 TOC. 활성 스터디 메타 + 책 list (주교재/부교재) + 펼친 책의 헤딩 트리 (5종 상태 아이콘 placeholder)
- `Workspace.tsx` 3-pane 재구성 `[Sidebar(260) | BookViewer | ChatPanel(380)]`. 사이드바·챗 collapse 토글 floating chevron 버튼
- 기존 상단 `BookList`는 사이드바로 흡수 (BookList.tsx 자체는 dead code로 잔존, 추후 정리)
- 단축키 prototype 정렬: `Mod+B`(사이드바 토글), `Mod+J`(챗 토글), `Mod+Shift+L`(라이브러리), `Mod+Shift+W`(워크스페이스), `Mod+/`(단축키 다이얼로그 — PR 36에서 hookup)
- `buildHeadingPlan` + 헬퍼들을 `src/lib/headingPlan.ts`로 분리 — BookViewer/StudySidebar 공유
- `uiStore`에 `sidebarOpen`/`chatOpen` 추가 (기본 true)

### Changed (PR 31 — v0.3 트랙 C 1단계: 디자인 토큰 + TopBar)
- `tokens.css` orange accent (oklch 0.62 0.18 25) + semantic colors(progress-*·intervention-l1/2/3·srs-*·cache-hit·queue-pending·validation) + density variants 추가 (D-070)
- TopBar prototype 100% 충실 재구성 — 브랜드 마크 + Library/Workspace 라우트 칩 + 단축키·Pomodoro·Wifi(오프라인 토글)·KO/EN(미지원, disabled)·Theme·Settings
- Memory/SRS/Recall 진입 버튼 TopBar에서 제거 — PR 33/34에서 SlideupTabs로 흡수 예정
- `uiStore`에 `density`(localStorage persist), `offline`(localStorage persist), `shortcutsOpen` 추가
- `App.tsx`가 `<html data-density="...">` attribute hookup
- ko.json `topbar.*`에 라우트·단축키·오프라인·언어 토글 라벨 추가

### Changed (PR 30 — v0.3 트랙 B: 새 스터디 마법사 재구성)
- 마법사 재설계 — Step 1(이름), Step 2(주교재 + 부교재), Step 3(요약+생성). 학습 목표·마감일 입력은 v0.3.1로 이관 (D-069)
- 슬러그 자동 도출 — 사용자에게 "슬러그" 단어 노출 X. 백엔드 `sanitize_to_slug`가 이름에서 디렉토리 안전 슬러그 생성
- 한국어 슬러그 허용 — `validate_slug` 갱신: OS 금지문자(`/ \ : * ? " < > |`)·control char·시작/끝 공백·점·Windows 예약어만 거부. v0.2 ascii 슬러그도 그대로 통과
- 충돌 처리 — `이름 (2)`, `이름 (3)` 형식 (`ensure_unique_slug`)
- 프론트엔드 `stripForbiddenChars` — 옵시디언 패턴으로 입력 시 즉시 strip
- `create_study` 시그니처 변경 — `slug` 인자 제거, name에서 자동 도출. 호출자(`api.createStudy`, `studyStore.create`) 갱신
- 부교재 `role_note` 컨텍스트 주입 — `SearchHit.book_role`/`book_role_note` 추가, `build_context_block`에서 부교재 hit에 `[부교재 — 역할]` prepend
- ko.json `wizard.*` 키 재구성 — 슬러그 텍스트 제거, 부교재 역할 메모 라벨 추가, 진행 메시지 키 추가
- 단위 테스트 +8 — 슬러그 검증·sanitize·충돌 카운터
- `decision-log.md` D-069 (v0.3 트랙 B 마법사 재구성 + 슬러그 한국어 그대로 + v0.3.1 이관) 추가

### Changed (PR 29 — v0.3 트랙 A: UI 텍스트 sweep)
- ko.json 전수 재작성 — 종결 어미 합니다체 일관 (D-068)
- 마크다운 문법(`*X*`), em dash(`—`), 본문 내 중점(`·`) UI 노출 제거
- 내부 메타 제거 — `v0.x`, `PR NN`, 내부 문서 참조(`release-pipeline.md`)
- 전문 용어 풀어쓰기 — `메타인지 제동`, `정규식 거짓 양성`, `Memory` UI 노출은 `학습 기록`으로
- placeholder 일반화 — `Rust 깊게 보기`, `rust-deep-dive`, `Programming Rust`, `Jim Blandy`, `Ch04` 같은 사례 placeholder 제거
- "회상 챌린지" → "회상 연습", "SRS 복습" → "복습", `Memory` UI 라벨 → "학습 기록"
- BookViewer PDF 에러·페이지 컨트롤 aria-label을 inline에서 i18n 키(`bookviewer.pdf_*`)로 분리
- `decision-log.md` D-067 (v0.3 슬라이스 정의) + D-068 (종결 어미 정책) 추가

### Added
- Tauri 2 + React 19 + TypeScript + Vite 스캐폴딩
- Tailwind v4 + shadcn/ui 설정 (`components.json`, `src/lib/utils.ts`)
- Pretendard Variable + Geist Mono 폰트
- 디자인 토큰 — `src/styles/tokens.css` (shadcn 기본 oklch · 라이트/다크)
- 경로 alias `@/*` → `src/*`
- `tests/` 디렉토리 골격
- `AppError` enum + `AppResult<T>` (`#[serde(tag = "kind")]` — TS union과 1:1)
- `tracing` 기반 로깅 — 일별 회전, 14일 보관, dev 빌드는 stderr 동시 출력
- 민감 정보 마스킹 함수 — `mask_api_key`·`mask_path`
- `rusqlite` (bundled) + `schema_version` 기반 마이그레이션 패턴
- v1 마이그레이션: `failed_llm_jobs` 큐 테이블
- `AppState` — `Mutex<Db>` + tracing `WorkerGuard` 보관
- 단위 테스트 14개 (AppError serde, 마이그 idempotent, 마스킹, CHECK 제약)
- API 키 보관 — `keyring` crate (OS 네이티브 키체인) + `zeroize`
- 6개 Tauri command — `api_key_check`·`api_key_set`·`api_key_delete`·`api_key_present`·`settings_read`·`settings_write`
- API 키 *형식* 검증 (sk-ant- prefix + 길이) — 실제 LLM 호출 검증은 PR 4
- 비밀 키는 절대 JS에 노출 X — `api_key_present`(boolean)만 외부 공개
- `Settings` 타입 — model·language·theme + 원자적 디스크 쓰기 (tmp + rename)
- Zustand `settingsStore` — 메모리 캐시 + 백엔드 동기화
- shadcn 컴포넌트 추가 — Button·Input·Label·Card·Tabs
- `Settings` 페이지 — Tabs 3 섹션 (API 키 / 모델 / 언어)
- `TopBar` + `Mod+,` 단축키로 Settings 토글
- 단위 테스트 +10 (Settings serde 5, 키 형식 검증 5)
- `LlmProvider` trait + `ChatRequest`/`ChatEvent`/`Usage` 타입 (D-005)
- `AnthropicProvider` — `reqwest` + `rustls`, `/v1/messages` POST + 스트리밍
- 직접 SSE 파서 (`SseParser`) — W3C 표준 1층만, 4종 에러 분류 (`[SSE-WIRE]`/`[SSE-EVENT-UNKNOWN]`/`[SSE-PAYLOAD-UNKNOWN]`/`[SSE-JSON]`)
- 백오프 — 429 한정 1s/2s/4s/8s ±20% jitter (8.6 절). 5xx·네트워크는 즉시 에러
- 모르는 SSE 필드(id·retry)·이벤트(`ping`)는 무시 — 통신 규격 forward-compat
- `MockProvider` — 미리 큐잉한 `ChatEvent` 흘려보내는 테스트용
- `chat_send` command — handle 즉시 반환 + `chat:chunk`·`chat:done`·`chat:error` events
- v0.1 가드: `study_slug != "default"` 또는 `context_section_id` 지정 시 `InvalidInput`
- `AppState`에 `current_file: Mutex<Option<String>>` (PR 5 FileViewer가 채움) + `llm: Arc<dyn LlmProvider>` 슬롯
- 단위 테스트 +20 (SSE 파서 10, Anthropic body·delta·usage·error·backoff 9, mock 1)
- `tauri-plugin-dialog` + `@tauri-apps/plugin-dialog` (파일 선택 다이얼로그)
- `commands/file.rs` — `file_open`·`file_close`·`file_current_content` (.md/.txt, 1MB 한도, UTF-8 검증)
- `Settings` 구조체에 `welcome_seen: bool` 추가 (default false)
- `react-markdown` + `remark-gfm` — GFM 마크다운 렌더 (LLM 응답 + 파일 뷰어)
- 마크다운 기본 스타일 — `tokens.css` `.markdown-body` (v0.3 syntax highlighting 후속)
- `react-i18next` + `i18next` — 한국어 번역 파일(`src/locales/ko.json`)·~50개 키
- PR 3 컴포넌트 한국어 문자열 *전체 추출* (Settings·ApiKeyInput·TopBar)
- shadcn `textarea` 컴포넌트 추가
- Zustand stores +3: `uiStore` (page·theme effective)·`fileStore`·`chatStore`
- `Welcome.tsx` — 첫 실행 환영 화면 (welcome_seen=false 시 표시)
- `Workspace.tsx` — FileViewer (좌) + ChatPanel (우) 2-pane
- `FileViewer.tsx` — 파일 다이얼로그·드래그앤드롭·메타·마크다운 렌더
- `ChatPanel.tsx` — 입력·전송·스트리밍 표시·키 보유 가드
- `ChatMessage.tsx` — 사용자/어시스턴트 분기·스트리밍 인디케이터·에러 배너
- `ThemeToggle.tsx` — system/light/dark 순환 + `prefers-color-scheme` listener
- `App.tsx` — 라우팅 (Welcome/Workspace/Settings) + 단축키(`Mod+,`·`Mod+L`·`Mod+Enter`) + drag-drop (`getCurrentWebview().onDragDropEvent`)
- `tests/fixtures/sample.md` — 검증용 샘플 교재
- `jobs.rs` — `failed_llm_jobs` 큐 헬퍼 (enqueue_or_update / list_jobs / fetch_payload / delete_job / is_retryable_error)
- 큐 dedup — UNIQUE(study_slug, job_type, payload_json) 충돌 시 attempts++ + error·last_attempt 갱신
- `chat_send`가 NetworkUnavailable·HTTP 5xx·SSE-WIRE 에러 시 자동 큐 적재 (4xx·AuthRequired는 적재 X)
- 새 commands: `retry_failed_job` / `list_failed_jobs` / `delete_failed_job`
- `chat:error` payload에 `job_id` 추가 (큐 적재된 경우)
- `ChatMessage`에 "다시 시도" 버튼 (job_id 보유 시) — 클릭 시 `retry_failed_job` 호출 + 새 어시스턴트 메시지 시작 + 기존 메시지의 job_id 비움
- v0.1 정책: *자동 워커 X* — 사용자 명시 재시도만. 자동 워커는 v0.2 (sequences.md SEQ-6 글자대로엔 못 미침)
- 단위 테스트 +8 (jobs: enqueue·dedup·list·fetch·delete·retryable 분류)
- `.github/workflows/test.yml` — PR / push 트리거 CI
- 3 OS 매트릭스 (ubuntu·macos·windows): `cargo fmt --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test`
- 별도 ubuntu 잡: `cargo audit` (taiki-e/install-action) · TS `pnpm typecheck`·`pnpm build`·`pnpm audit --audit-level=high`
- `Swatinem/rust-cache@v2` 빌드 캐싱
- concurrency 그룹 — 같은 브랜치 push 연달아 시 이전 실행 자동 취소
- v0.2 도입 예정: `vitest` (`pnpm test:unit`)·`eslint` (`pnpm lint`)
- v0.3+ Playwright E2E는 `nightly-e2e.yml`로 분리

### Changed
- `src-tauri/Cargo.lock`을 git 추적 시작 (Tauri 앱 = binary, 재현 빌드 + `cargo audit` 재현성)
- DB 마이그 v2 — `studies`·`chat_messages`·`books` 테이블 추가, `failed_llm_jobs`에 FK + ON DELETE CASCADE 부착 (CREATE+COPY+RENAME 패턴)
- `chat_send`의 `study_slug` 가드 제거 — 활성 스터디 슬러그 그대로 사용 (실존 검증 + chat_messages 영속)
- `studies.is_active` 컬럼 + partial unique index = 활성 스터디 source of truth (메모리 캐시는 `AppState.active_study`)

### Fixed (v0.2.1 PR 28.1 hotfix) — ChatPanel가 auth_mode 무시하고 API 키 강제
- 증상: CLI 모드 전환 + CLI 설치/로그인 완료한 사용자가 챗 화면에서 "API 키 필요" UI를 보고 send 버튼이 작동 안 함
- 원인: `ChatPanel.tsx:78`이 `auth_mode` 무시하고 무조건 `apiKeyPresent(activeProvider)` 호출. CLI 모드에선 keyring에 키가 없으니 `hasKey=false`로 떨어져 UI가 "API 키 입력" 화면으로 박힘
- 수정: `auth_mode === "cli"`일 때 keyring 체크 건너뛰고 `hasKey=true`로 처리. CLI 인증 상태 검증은 백엔드 chat_send에 위임

### Fixed (v0.2.1 PR 28 hotfix) — CLI 모드 전환 시 provider rebuild 누락
- 증상: Welcome → Claude 카드 클릭 → CLI 설치/로그인 완료 → 챗 시도 → "API 키 연결 필요" 에러
- 원인: `settings_write`가 auth_mode=cli로 갱신할 때 *그 시점엔 아직 CLI 미설치*라 `build_provider`가 `Err(CliMissing)`을 반환 → settings는 저장됐지만 `AppState.llm`은 옛날 `AnthropicProvider` 그대로 박힘. 이후 `cli_install_provider`/`cli_login`이 성공해도 누구도 provider rebuild 안 함.
- 수정:
  - `lib.rs::try_rebuild_llm(&state)` 신규 헬퍼 — 현재 settings 기준으로 build 시도, 실패 시 기존 provider 유지(fail-soft)
  - `settings_write`: build_provider 실패해도 에러 안 던짐 — settings 저장은 성공
  - `cli_install_provider`: 설치 성공 후 `try_rebuild_llm` 호출
  - `cli_login`: Anthropic/Codex 로그인 성공 후 `try_rebuild_llm` 호출
  - `cli_auth_status_claude`/`cli_auth_status_gemini`/`cli_auth_status_codex`: `logged_in=true`면 `try_rebuild_llm` 호출 (외부 터미널 인증 케이스 회복)
  - `build_provider` 자체도 fail-soft: CLI build 실패하면 ApiKey 어댑터로 fallback (앱 startup 보장)
- 영향: PR 28 적용 전 사용자는 앱 재시작 또는 Settings → CLI 연결 다이얼로그 재진입으로 recovery 가능

### Added (v0.2.1 PR 27) — 첫 실행 onboarding 재작성 + Settings Advanced 탭
- `Welcome.tsx` 전면 재작성 — "이미 구독 중이세요?" 섹션을 1순위로 노출. Claude(추천)/Gemini(무료)/Codex 카드 클릭 시 `auth_mode=cli` + `active_provider` 저장 후 `CliSetupDialog` 띄움. onComplete 시 자동으로 `welcome_seen=true` + 워크스페이스 이동.
- "구독 없이 API 키로 직접 시작 (Advanced)" 링크 — 클릭 시 `auth_mode=api_key` 설정 후 Settings로 이동
- Settings에 새 "Advanced" 탭 추가 — API 키 직접 입력 카드 이동 (이전엔 Provider 탭 하단에 있었음)
- Provider 탭은 이제 인증 방식(auth_mode) + 프로바이더 선택만 — 깔끔한 의도 분리
- 신규 locale 키 — `welcome.cli.{section_title,section_desc,*_title,*_sub,*_badge}`, `welcome.advanced_link`, `settings.tabs.advanced`, `settings.advanced.api_key_desc`
- 결정 (PR 27): #1 Welcome은 "구독 연결" 중심 — 무구독자는 Gemini 무료 티어 카드로 시작 / #2 API 키 입력은 Advanced 탭으로 강등하되 *제거 X* (사용자 선택권 보장) / #3 신규 사용자 기본 auth_mode는 ApiKey (settings.json 부재 시 default) — Welcome에서 *명시적으로* CLI 선택해야 전환

### Added (v0.2.1 PR 26) — Codex CLI 브릿지
- `llm/codex_cli.rs` — `codex exec --json --model <m> "<query>"` 자식 프로세스 어댑터
- JSONL 파서 — `item.completed{item:{type:"agent_message",text}}` → `ChatEvent::TextDelta`, `turn.completed{usage:{input_tokens,cached_input_tokens,output_tokens,reasoning_output_tokens}}` → `ChatEvent::Done`, `turn.failed`/`error` → `AppError::CliRuntime`
- `agent_reasoning`·`command_execution`·`plan_update` 등 다른 item.type은 무시 (LLM 텍스트 응답만 사용)
- `cli_auth_status_codex` 커맨드 — `codex login status` exit code (0=인증) 활용
- `cli_login` Codex 분기 — `codex login` 직접 spawn (브라우저 OAuth) / `codex login --with-api-key` (console 모드)
- 시스템 프롬프트는 user 본문 앞에 prepend (Gemini와 동일 패턴)
- `build_provider`에 OpenAI → `CodexCliProvider` 분기 활성화 (PR 24/25 인프라 그대로 재사용)
- 프론트 — `CliSetupDialog`의 openai 분기 활성화, `cliAuthStatusCodex` API 추가
- 단위 테스트 +8 (agent_message·reasoning skip·command_execution skip·turn.completed·turn.failed·thread.started·turn.started·malformed)
- 결정 (PR 26): #1 Codex login은 직접 spawn(브라우저 OAuth) 가능 — Gemini와 달리 TerminalInstruction 필요 없음 / #2 agent_message만 통과 (reasoning/command은 chat UI에 노이즈) / #3 cached_input_tokens → cache_read_input_tokens 매핑

### Added (v0.2.1 PR 25) — Gemini CLI 브릿지
- `llm/gemini_cli.rs` — `gemini "<query>" -o stream-json -m <model>` 자식 프로세스 어댑터
- stream-json 라인 파서 — `message{role:"assistant",content,delta:true}` → `ChatEvent::TextDelta` (진짜 델타·차분 계산 X), `result{status,stats:{input_tokens,output_tokens,cached}}` → `ChatEvent::Done`
- 시스템 프롬프트는 user 본문 앞에 `<sys>\n\n---\n\n<query>` 형태로 prepend (CLI 자체 시스템 옵션 부재 회피)
- `cli_auth_status_gemini` 커맨드 — 별도 status 명령 부재 → 짧은 ping(`gemini . -o json -m flash`) exit code로 인증 추정
- `cli_login` Gemini 분기 — 비대화형 login 명령이 마땅치 않아 `CliLoginOutcome::TerminalInstruction { command, hint }` 반환
- 프론트 — `CliSetupDialog` 일반화: anthropic/gemini/openai 분기 + TerminalInstruction 박스 + `recheck` 버튼
- `build_provider`에 Gemini → `GeminiCliProvider` 분기 추가, `locate_required` 헬퍼로 중복 제거
- 단위 테스트 +6 (assistant 델타·user 메시지 skip·result success/failure·init skip·malformed JSON skip)
- 결정 (PR 25): #1 Gemini auth status는 ping으로 추정 — `~/.gemini/oauth_creds.json` 직접 검사 회피 / #2 시스템 프롬프트는 prepend (CLI에 명시 옵션 없음) / #3 비대화형 login 부재 → 사용자 터미널 안내로 우회

### Added (v0.2.1 PR 24) — CLI 인프라 + Claude Code 브릿지
- D-066 결정 — v0.2.1 인증 경로: 공식 CLI subprocess가 메인, API 키 직접 입력은 Advanced 백업 (구독 그대로 활용 박탈감 해소)
- `runtime.rs` — Node/npm PATH 감지 + `~/.airis/npm` 전용 prefix (sudo 회피)
- `cli_install.rs` — `npm install -g --prefix=<airis>` 래퍼 + 프로바이더↔패키지 매핑 (`@anthropic-ai/claude-code`·`@google/gemini-cli`·`@openai/codex`)
- `llm/claude_cli.rs` — Claude Code 자식 프로세스 어댑터: `claude -p ... --output-format stream-json --verbose --no-session-persistence --tools "" --setting-sources ""` + cwd를 app_data_dir로 격리 (사용자 CLAUDE.md 자동 발견 차단)
- stream-json JSONL 파서 — `assistant` 이벤트 누적 차분 → `ChatEvent::TextDelta`, `result` → `ChatEvent::Done` (usage 매핑)
- `tokio::process::Command` + `kill_on_drop(true)` + ChildGuard로 좀비 프로세스 방지
- 신규 Tauri 커맨드 5종 — `cli_runtime_detect`·`cli_status`·`cli_install_provider`·`cli_auth_status_claude`·`cli_login`
- `claude auth status` JSON 파싱 → `ClaudeAuthInfo { logged_in, auth_method, subscription_type, email }` 노출
- `Settings.auth_mode` (ApiKey/Cli) + `cli_versions: HashMap` 필드. `settings_write` 시 active_provider 또는 auth_mode 변경되면 build_provider rebuild
- 신규 에러 4종 — `NodeMissing`·`CliMissing`·`CliAuthRequired`·`CliRuntime`
- `CliSetupDialog.tsx` — 3단계(런타임 감지 → 설치 → 로그인) 진행 + 구독/콘솔 로그인 분기 + 에러 표시
- Settings → 프로바이더 탭 상단에 `auth.mode_card` 추가 (CLI 추천, API 키 백업)
- 단위 테스트 +8 (`cli_binary_path_unix`·`pkg_for_provider_matches_expected`·claude_cli JSONL 파서 5종 등)
- 디자인 — `design/v0.2.1_HANDOFF.md` 신규, `decision-log.md` D-066 추가
- 결정 (PR 24): #1 auth_mode 기본 ApiKey (v0.2 호환) — Cli 전환은 Settings/PR 27 Welcome에서 / #2 Anthropic만 우선 구현, Gemini/Codex는 PR 25/26 / #3 사용자 환경 격리 = `--tools "" --setting-sources "" --no-session-persistence` + cwd 강제 / #4 npm 전용 prefix `~/.airis/npm` (sudo 회피)

### Added (v0.2 PR 23) — v0.2 완성 🎉
- 자동 큐 워커 — `jobs::enqueue_or_update`에 exponential backoff next_retry_at 적용 (1m/2m/4m/8m, 4회 후 NULL → 수동만)
- `list_due_jobs` command — `next_retry_at <= NOW`인 잡 반환
- 프론트 자동 워커 — App.tsx 30초 polling, retryFailedJob 자동 호출. 결과는 chat:done 흐름으로 자연 통합
- F14.1 인앱 업데이트 알림 — `commands/updates.rs::check_for_update` (GitHub Releases API + SemVer 비교)
- 앱 시작 시 1회 + 24h throttle (localStorage `airis:update:last_check`)
- `UpdateDialog.tsx` — 새 버전 정보 + release notes preview + tauri-plugin-opener로 GitHub 페이지 open
- F14.2 SHA256 검증 표시 — release notes에 "sha256" 키워드 있으면 안내 표시 (release-pipeline.md 무서명 정책)
- 단위 테스트 +3 (semver newer·pre-release suffix·invalid)
- 결정 (PR 23): #1 자동 retry UX = chat:done 흐름 그대로 (별도 토스트 X — 자연 통합) / #2 업데이트 = 시작 시 + 24h throttle

### Added (v0.2 PR 22)
- F7.7 회상 챌린지 — `commands/recall.rs` (사용자가 챕터 핵심 적기 → paragraphs에서 빈도 top-8 키워드 추출 → 매치 비교)
- 통과 임계 60% (PASS_THRESHOLD) — 통과 시 자동 SRS 카드 생성 (F8.2 활성)
- DB 마이그 v7 — `recall_challenges` 테이블 (db-schema.md 그대로). expected/present/missing JSON 보관
- 키워드 추출 휴리스틱 — 영문/한글 ≥2자, 공백 분리 token, 빈도 정렬, 한·영 stop words 제외
- `RecallPanel.tsx` 슬라이드업 — 챕터 ref + textarea + 평가 결과 (expected/present/missing 색상별 badge)
- TopBar Lightbulb 아이콘 + `Mod+R` 단축키 (`uiStore.recallOpen`)
- 단위 테스트 +4 (top keywords 빈도·stop words 필터·한국어·normalize)
- 결정 (PR 22): F7.1 트리거 임계 = *모든 챕터 명시만*. 챕터 신뢰도 기반 자동 트리거(L1/L2)는 챕터 신뢰도 데이터 도입 후 v0.3+
- LLM 기반 평가는 v0.3+ (현재는 결정적 휴리스틱 — 비용 0)

### Added (v0.2 PR 21)
- F8 SRS — SuperMemo SM-2 알고리즘 (`commands/srs.rs::sm2_step` pure 함수, e_factor floor 1.3, 실패 시 reset)
- DB 마이그 v6 — `srs_cards` 테이블 (db-schema.md 그대로). FK study_slug, due_date 인덱스
- commands: `srs_add_card`·`srs_list_due`·`srs_review_card`·`srs_delete_card`
- 자동 due_date 계산 — std로 epoch → ISO 날짜 (chrono crate 의존 X). pomodoro의 `days_to_ymd_pub` 재사용
- `SrsPanel.tsx` 슬라이드업 — due 카드 차례차례, CSS transform rotateY로 flip 애니메이션 (framer-motion 도입 X)
- 평가 4단계 (again=0 / hard=3 / good=4 / easy=5) → SM-2 quality 매핑
- 카드 추가 다이얼로그 — front/back/section_ref 수동 입력
- TopBar Layers 아이콘 + `Mod+K` 단축키 (`uiStore.srsOpen`)
- 단위 테스트 +5 (sm2 first pass·second pass·실패 reset·기하 성장·e_factor floor)
- 결정 (PR 21): 카드 flip = CSS만 (A). framer-motion 도입은 v0.3+. 자동 카드 생성(F8.2)은 PR 22 회상 챌린지 통과 시 활성

### Added (v0.2 PR 20)
- F9 Pomodoro 타이머 — `commands/pomodoro.rs` (wall-clock 기반, started_at + duration_min만 저장 → OS sleep/wake에 정확)
- DB 마이그 v5 — `pomodoro_cycles` 테이블 (v2 누락분 보강). FK study_slug, phase CHECK, 인덱스
- AppState `pomodoro: Mutex<Option<PomodoroSession>>` — 단일 활성 세션
- start_pomodoro·stop_pomodoro·get_pomodoro_state commands
- 사이클 종료 시 pomodoro_cycles INSERT (completed/interruption 메타)
- `PomodoroPanel.tsx` 미니 패널 (우하단 floating) — 1초 polling, 25/5분 기본, 자동 만료 감지 + 자동 stop
- TopBar Timer 아이콘 + `Mod+Shift+P` 단축키 (`uiStore.pomodoroOpen`)
- 결정 (PR 20): wall-clock 기반 (B). OS 네이티브 알림은 v0.3+, 인앱 토스트만. 자동 세션 추적(F6.1)도 v0.3+
- 단위 테스트 +3 (db v5 1 + pomodoro persist 1 + format_iso 1 + leap year 1)

### Added (v0.2 PR 19) — v0.2c 시작
- F2.8/F12.2 stale 감지 — `commands/book::check_stale` (활성 스터디 모든 책의 source_path 현재 sha256 vs books.file_hash 비교, missing/changed 보고)
- `commands/book::reindex_book` — 변경된 파일의 hash·size 갱신 + start_indexing 흐름 호출
- `bookStore.staleByBookId`·`reindex`·`checkStale` — refresh 시 자동 stale 검사
- BookList 카드 stale 배지 (changed/missing) + 재인덱싱 버튼 (RotateCw 아이콘 + spinner)
- 단순화 결정 (PR 19): L3 폰트 클러스터링은 PR 19.5 (또는 v0.3)로 이연 — pdfium-render 폰트 API 검토 비용 큼. 회귀 테스트(F12.4/F12.5)도 v0.3+

### Added (v0.2 PR 18) — v0.2b 마무리
- DB 마이그 v4 — `intervention_signals`·`search_history`·`consistency_check_log` 테이블 추가 (db-schema.md 그대로)
- F7.2 반복 검색 감지 — `search_sections` 호출 시 search_history 적재 + query_norm 정규화(소문자·token sorted) + 30분 윈도우 N=3회 누적 시 `intervention_signals.repeat_search` 적재
- F12.1 Memory active 모순 검사 — `commands/consistency.rs` (Preferences/Corrections active 항목 키워드 겹침 휴리스틱). `memory_write` 후 자동 호출 → `consistency_check_log` 기록
- 정책: PR 18 시점엔 *데이터 누적*만, UI alert·signals 노출은 v0.3+
- 단위 테스트 +5 (db v4 1 + consistency 4)
- *결정 포인트 X* — 강도 명명은 PR 15에서 이미 confirm/auto/off로 박힘

### Added (v0.2 PR 17)
- F4.4 응답 검증 — `commands/validation.rs` (Memory.Corrections active 항목의 부정 패턴 추출 → 응답 매치 시 ViolationHit). 결정적 정규식만, LLM 검증은 v0.3+
- chat:done 직후 `emit_violations` hook — `chat:violation` event 발사
- chatStore `attachViolations` — 진행 중/직전 어시스턴트 메시지에 violations 첨부
- ChatMessage 노란 배너 — 위반 의심 항목 표시 (응답은 그대로, 거짓 양성 가능 명시)
- F4.5 3층 응답 — system prompt에 형식 안내 (요약 / 본문 인용 [1] / 더 알아보려면)
- ChatMessage `[1]`·`[2]` 인용 마커 인라인 강조 (badge 형태). 클릭 점프는 v0.3+
- 단위 테스트 +5 (validation: 위반 감지·미위반·resolved 무시·other section 무시·extract 안전성)
- 결정 (PR 17): 검증 위반 시 = 노란 배너 + 응답 그대로 (A). 강도 따른 재생성은 v0.3+

### Added (v0.2 PR 16)
- F10.5 `memory::compress` — 5섹션에서 *active 항목만* 추출 → L1(Preferences+Corrections, 2000자) + L2(Progress+Meta+Goals, 4000자)
- F10.6 `chat_send` 자동 주입 — Memory L1·L2를 system prompt 끝에 합성. 활성 섹션·검색 결과는 user message에
- D-036 prompt cache 활성 (Anthropic) — `ChatRequest.cache_breakpoints: Vec<CacheBreakpoint>` (System / Message(idx))
- AnthropicProvider build_request_body — cache_breakpoints 활용해 system block을 `[{type:text, text, cache_control:{type:ephemeral}}]` 형태로 wrap. 메시지 인덱스 cache_breakpoint도 동일 패턴
- OpenAI는 자동 prefix 캐싱(서버 측)이라 cache_breakpoints 무시. Gemini cachedContents는 v0.3+로 이연 (handoff 결정 #3)
- 단위 테스트 +5 (memory compress 3 + anthropic cache_control 2)
- 결정 (PR 16): #1 캐시 위치 메모리 / #2 cache_breakpoints 인덱스 (B) / #3 Gemini v0.3+

### Added (v0.2 PR 15)
- F10.3 발화 트리거 감지 — `commands/triggers.rs` 정규식 사전 (preference / correction / goal 분류, 한글·영문 패턴)
- `memory_detect_triggers`·`memory_apply_trigger` commands — 사용자 발화 → 트리거 hit → Memory 5섹션 자동 append
- `memory.rs::append_to_section` 헬퍼 — heading 발견 시 *그 섹션 안에* 항목 박음, 부재 시 새 섹션 생성
- `(active, since YYYY-MM-DD)` prefix 자동 — std로 epoch → ISO 날짜 (chrono crate 의존 X)
- F13.6 `Settings.intervention_level` (Confirm·Auto·Off) — 트리거 감지·갱신 정책
- `TriggerDialog.tsx` 1회 확인 다이얼로그 (우하단 floating, 매치 발화 + 추가될 항목 + 추가/건너뛰기)
- ChatPanel 통합 — 사용자 발화 직후 detect 호출 + 강도별 분기 (confirm 다이얼로그 / auto 즉시 적용 / off 비활성)
- Settings "강도" 탭 추가 — 3 옵션 라디오
- 의존성 추가: `regex` 1
- 단위 테스트 +9 (triggers 7 + memory append 2)
- 결정 (PR 15): 트리거 패턴 사전 = *코드 박음* (A). triggers.toml 외부 파일은 v0.3+ 검토

### Added (v0.2 PR 14)
- F10 Memory.md 표준 도입 — 사용자 성향·진도·이해도 누적 영역 (시스템 자동 갱신, 사용자 직접 편집 가능)
- `commands/memory.rs` — `MemoryDoc`(study·updated·body), 5섹션 헤딩 상수, frontmatter 파서/빌더 (Overview와 같은 정책)
- `memory_read`·`memory_write` commands — 원자적 쓰기(`.tmp` → atomic rename, SEQ-8) + mtime+sha256 fingerprint
- 외부 편집 감지 — 마지막 write fingerprint 모듈 단위 보관, read 시 비교 → `external_edited` 플래그
- 첫 read 시 default template 자동 반환 (5섹션 헤딩 포함)
- `components/MemoryEditor.tsx` 슬라이드업 패널 — 단일 textarea + 저장 + 외부 편집 경고 + 다시 불러오기
- TopBar에 Brain 아이콘 진입 + `Mod+M` 단축키 (`uiStore.memoryOpen` 글로벌 floating)
- 단위 테스트 +7 (parse round-trip, 폴백 슬러그, write/read round-trip, default template, 외부 편집 감지, fingerprint 매칭, 원자성 — tmp 잔류 X)
- 결정 (PR 14): 외부 편집 감지는 *로드 시점 mtime+hash 비교* (B). fs watcher는 v0.3+. Stronghold 폴백은 PR 14.5로 분리

### Added (v0.2 PR 13)
- 다중 LLM 프로바이더 — Anthropic + OpenAI + Gemini (D-005 부분 supersede)
- `settings::Provider` enum + `Settings.active_provider`·`models: HashMap<Provider, model>`
- 키 형식 검증 분기 — `sk-ant-` / `sk-` / `AIza` (각 prefix·최소 길이)
- `llm/openai.rs` — Chat Completions API + SSE + `[DONE]` 종료 + stream_options.include_usage
- `llm/gemini.rs` — `:streamGenerateContent?alt=sse` + `x-goog-api-key` + safety/blockReason 로그
- `AppState.llm: Mutex<Arc<dyn LlmProvider>>` — Settings.active_provider 변경 시 새 instance 교체. 진행 중 chat은 자기 Arc clone으로 끝까지 완료 (결정 #4)
- `lib::build_provider` 헬퍼 — Provider → Provider 인스턴스
- Settings UI 갱신 — "프로바이더" 탭 (활성 라디오 + 3개 카드 키 입력) / "모델" 탭 (활성 프로바이더 모델 셀렉터) / "언어" 탭
- ApiKeyInput placeholder 분기 — `PROVIDER_KEY_HINT`로 prefix·placeholder 표시
- ChatPanel 활성 프로바이더 키 검사 — `apiKeyPresent(active_provider)`
- 단위 테스트 +20 (openai 5 / gemini 7 / settings 4 / commands/settings 4)
- 결정 (handoff): #1 단일 active / #2 safety는 배너+응답 그대로(PR 17) / #3 정적 모델 목록 / #4 진행 중 챗 그대로 완료

### Added (v0.2 PR 12.6)
- 인앱 PDF 뷰어 — `pdfjs-dist` 5.7 + Tauri Asset Protocol 통합
- `tauri.conf.json` `app.security.assetProtocol` 활성 + scope (`$HOME/**`·`$APPDATA/**`·`$DOCUMENT/**`·`$DOWNLOAD/**`)
- Tauri `protocol-asset` feature 활성 (Cargo.toml)
- `BookContent.source_path` 추가 — PDF는 빈 content + source_path만, pdfjs가 `convertFileSrc`로 직접 로드
- BookViewer에 `PdfContent` 컴포넌트 — 페이지 캔버스 렌더 + 페이지 네비 (이전/다음 버튼·번호 입력)
- pdfjs worker 등록 — Vite `?url` 패턴
- `activeBookStore` `pendingPage` + `consumePendingPage` — 검색 결과 클릭 시 PDF 페이지 점프
- BookList — PDF 책 카드도 클릭 가능 (이전 disabled 가드 제거)
- 의존성 추가: `pdfjs-dist` 5.7
- 보안 표면 변경 명시 — assetProtocol scope 4개 home/data 디렉토리. v0.3에서 *동적 scope* (등록한 책만)으로 좁히기 검토

### Added (v0.2 PR 12.5)
- PDF 인덱싱 활성 — `start_indexing`이 PDF 분기 처리 (`parsers::pdf::parse` 호출)
- PDFium binary 동봉 — `scripts/setup-pdfium.sh` (Linux/macOS) + `pdfium-binaries` chromium/6996 다운로드 + `src-tauri/resources/pdfium/lib/`에 압축 해제
- `package.json` `pdfium:setup` 스크립트 + README 안내
- `tauri.conf.json` `bundle.resources` — pdfium lib·include·README placeholder 명시 (Tauri glob 매칭 안정)
- AppState `pdfium_lib_dir: Option<PathBuf>` 추가 — Tauri `resource_dir` 기반 자동 탐지. None이면 PDF 인덱싱 명시 안내 후 graceful skip
- `parsers::pdf::extract_from_text_fallback` 갱신 — 챕터 위치별 *페이지 본문 concat*. 챕터 없는 PDF는 단일 `Ch01`에 책 전체 본문 (검색 가능성 보존)
- AddBookDialog — PDF도 자동 인덱싱 호출. 안내 문구 정리 (시각 뷰어는 PR 12.6)
- `.gitignore` — `src-tauri/resources/pdfium/*` 추적 X, `PDFIUM_README.txt` placeholder만 유지

### Added (v0.2 PR 12)
- `BookViewer.tsx` — MD/HTML 책 뷰어. ReactMarkdown로 헤딩 렌더 시 클릭 가능. 활성 헤딩 시각 강조. 검색 결과·인용 클릭 시 ref 기반 anchor scroll
- HTML은 sandbox iframe + srcDoc (백엔드 ammonia sanitize와 이중 안전)
- TS heading slug 규칙 — Rust `parsers/slug.rs` 미러 (영/한 챕터 정규식·CJK 보존·dedupe)
- 백엔드 `book_read_raw` command — 책 raw content + format 반환 (PDF는 PR 12.5)
- 백엔드 `set_active_section`·`clear_active_section`·`get_active_section` commands + AppState `active_section` 캐시
- `chat_send` 컨텍스트 우선순위 변경 — *활성 섹션* (paragraphs WHERE book_id+section_path) → FTS5 검색 폴백 → current_file 폴백
- `BookList` 검색 입력 + 인라인 dropdown — 디바운스 300ms, 5 결과, FTS5 snippet `<<>>` → `<mark>` 변환
- `BookList` 책 카드 클릭 → `activeBookStore.open` → BookViewer 진입. 활성 책 시각 강조
- `Workspace` 라우팅 — 활성 책 있으면 BookViewer, 없으면 FileViewer (v0.1 fallback)
- 검색 결과 클릭 → `activeBookStore.jumpTo` (책 열기 + 섹션 점프 + 활성 박기) 일체 흐름
- `activeBookStore` (Zustand) — bookId·content·sectionPath·pendingScrollPath
- D-064 PR 12 정신 명시: 사용자 *명시 클릭*만 활성 — 자동 스크롤 추적 X (예측 가능성 우선)

### Added (v0.2 PR 11)
- DB 마이그 v3 — `paragraphs` (검색 단위, 섹션을 ~500자 청크로 분할) + `paragraphs_fts` (SQLite FTS5 virtual table, unicode61 tokenizer) + 자동 동기화 트리거 (INSERT/UPDATE/DELETE)
- `index/chunker.rs` — 문장 경계 보존 청킹 (한국어 종결·영어 마침표·줄바꿈, hard max 강제 분할)
- `index/keyword.rs` — 트랜잭션 단위 paragraphs rebuild (FTS는 트리거가 자동)
- `commands/book.rs` — `add_main_book`·`add_sub_book`·`list_books`·`remove_book`·`start_indexing`. SHA-256 파일 해싱(sha2 crate). PDF는 PR 12로 이연 (인덱싱 시 안내 에러)
- `commands/search.rs::search_sections` — FTS5 MATCH (prefix 와일드카드 자동) + bm25 점수 + Top-K=5 + snippet 하이라이트
- `chat_send` 컨텍스트 자동 주입 — current_file 본문 우선, 없으면 활성 스터디 책에서 FTS5 검색 → Top-K 섹션 자동 컨텍스트
- 마법사 단계 3 추가 — 완료 안내 (책 등록은 워크스페이스에서)
- `components/AddBookDialog.tsx` — 파일 선택 + 메타 입력 + 등록 + 인덱싱 + 진행률 (`index:progress` event)
- `components/BookList.tsx` — 워크스페이스 상단 책 목록 + "책 추가" 버튼 + 삭제 + indexed 상태 표시
- `bookStore` (Zustand) — books·refresh·add·remove·startIndexing
- 단위 테스트 +16 (chunker 6 + keyword 3 + search 4 + db v3 2 + chunker hard split 1)
- 의존성 추가: `sha2` 0.10
- D-018·D-060 supersede + 새 D-064/D-065 추가 (v0.2 임베딩·하이브리드 미도입, v0.3 검토)

### Added (v0.2 PR 10)
- 책 파서 라이브러리 (`src-tauri/src/parsers/`) — F2 결정적 코어. PR 11 commands에서 호출 들어오면 활성화
- `parsers/types.rs` — `Section`·`SectionLevel`(Chapter/Section)·`ParsedBook`·`BookMetadata`·`BookFormat`. 4계층 모델 (L1 Book / L2 Chapter / L3 Section / L4 Paragraph는 PR 11)
- `parsers/slug.rs` — 챕터 번호 정규식(영문 "Chapter N"·"Ch.N"·한글 "제 N 장"·"N장") + 한글 보존 path 슬러그 + 충돌 시 `-2`·`-3` suffix
- `parsers/markdown.rs` — `pulldown-cmark` 기반 ATX/Setext heading 추적. h1=Chapter, h2~h6=Section. h1 부재 시 첫 h2 챕터 승격. 본문은 heading 사이 raw 마크다운
- `parsers/html.rs` — `ammonia` sanitize → `scraper` heading 추출. script·on* 제거 + strong·em·code 보존. 텍스트 평탄화로 본문 추출
- `parsers/pdf.rs` — `pdfium-render` 기반 페이지 텍스트 추출 + 챕터 정규식 폴백. PDFium binary는 runtime 동적 로드 (앱 번들 동봉)
- 결정 (PR 10): PDF 엔진 = pdfium-render (한국어 정확도 1순위), 섹션 ID = `{book-uuid}/Ch04/§State` 의미 path
- PDF Outline(북마크) 기반 L1 추출은 PR 19로 이연 (pdfium-render 0.8 API 검토 추가 필요)
- 의존성 추가: `pulldown-cmark` 0.12 + `scraper` 0.21 + `ammonia` 4 + `pdfium-render` 0.8
- 단위 테스트 +23 (slug 8: 영/한 챕터·padding·section path·dedupe·display label · markdown 6: h1/h2·한글·h2 승격·dedup·body 추출·empty · html 5: 계층·body·sanitize 2종·empty · pdf 3: chapter 폴백·empty·dedup)

### Added (v0.2 PR 9)
- F1 Library 페이지 (`pages/Library.tsx`) — 카드 그리드, 활성 강조, 정렬(활성 우선·last_opened DESC), 카드 클릭 시 활성 전환 + 워크스페이스 이동
- 새 스터디 마법사 (`pages/NewStudyWizard.tsx`) — 한 화면 + step indicator (옵션 A 결정), 2단계 (이름·슬러그 / stated_goal·deadline). PR 10·11에서 단계 추가 예정
- `components/StepIndicator.tsx` — 진행률 표시. 옵션 B(슬라이드) 도입 시 그대로 재사용
- 삭제 확인 다이얼로그 — 카드별 삭제 + 한 번 더 확인. 백엔드는 삭제 후 다른 스터디로 자동 활성 전환
- TopBar 활성 스터디 라벨 + Library 진입 버튼(`Mod+B`)
- 백엔드 `commands/overview.rs` — Overview.md 영속 (`{data_dir}/studies/{slug}/Overview.md`). frontmatter 파서/빌더 (단순 key:value, 외부 crate X). 원자적 쓰기(`.tmp` + rename)
- 새 commands: `study_overview_read`·`study_overview_write_meta`. `create_study` 시 Overview.md 템플릿 자동 생성 (실패는 비치명)
- `studyStore` 확장 — `list`·`refreshList`·`create`·`remove`. Library에서 사용
- 단위 테스트 +8 (overview: round-trip·unknown 키 무시·인라인 주석·따옴표·디스크 round-trip·patch_meta body 보존·default fallback)

### Added (v0.2 PR 8)
- v2 마이그레이션 SQL — v0.1 사용자의 기존 큐 슬러그를 자동 보존(FK 위반 방지)
- `commands/study.rs` — `list_studies`·`create_study`·`select_study`·`delete_study`·`get_active_study` 5 commands + 슬러그/이름 검증
- `ensure_active_or_bootstrap_default` — 부팅 시 활성 스터디 없으면 'default' 자동 생성·활성화 (v0.1 사용자도 끊김 없이 챗 가능)
- `chat_history` command — 활성 스터디의 최근 메시지 시간순 반환 (cursor 페이징)
- 토큰·모델 메타 영속 (creation_tokens·output_tokens·cache_hit_tokens·model)
- 프론트엔드 `studyStore` (Zustand) — 활성 스터디 캐시 + `select`
- `chatStore.hydrate` — 부팅 시 영속 메시지 복원
- 단위 테스트 +16 (Rust: study slug/name 검증·active uniqueness·bootstrap·cascade·chat_history; vitest: ApiKeyInput·ChatMessage 11개)
- vitest + jsdom + @testing-library/react/jest-dom/user-event 도입 — `pnpm test:unit`
- eslint flat config (typescript-eslint + react-hooks + react-refresh) — `pnpm lint --max-warnings 0`
- `.github/workflows/test.yml` 갱신 — TS 잡에 `pnpm lint`·`pnpm test:unit` 단계 추가
