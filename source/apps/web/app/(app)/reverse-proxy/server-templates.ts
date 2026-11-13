import { emptyDirectiveState, type DirectiveEditorState } from "@repo/presentation/nginx/directive-state";
import type { LocationInput } from "@/components/reverse-proxy/server-form-vm";

/**
 * Reverse-proxy 생성 템플릿(코드 상수). defaults는 ServerForm의 defaultValues가
 * 이해하는 형태 그대로라 변환 계층이 없고, 모든 디렉티브는 기존
 * DIRECTIVE_ALLOWLIST 안이므로 저장 시 기존 검증 경로를 그대로 통과한다.
 * 갤러리 미리보기가 stateToDirectives+renderReverseProxy를 실호출하므로
 * 갤러리가 렌더되면 이 데이터 전체가 검증·렌더 경로를 통과했다는 증명이 된다.
 * RSC 직렬화로 클라이언트는 매 마운트마다 복사본을 받는다(공유 참조 없음).
 */
export interface ServerTemplate {
  id: string;
  name: string;
  description: string;
  tags: string[];
  /** 미리보기 렌더용 대표 server_name placeholder. */
  previewServerName: string;
  defaults: {
    listen: number;
    /** 단일 모드 placeholder 대상. locations 모드면 생략. */
    proxyPass?: string;
    locations?: LocationInput[];
    serverDirectives: DirectiveEditorState;
    locationDirectives?: DirectiveEditorState;
  };
}

function ds(partial: Partial<DirectiveEditorState>): DirectiveEditorState {
  return { ...emptyDirectiveState(), ...partial };
}

/**
 * 표준 리버스 프록시 헤더 세트. Host 변수만 앱별로 다르다(Nexus는 $host,
 * GitLab 공식 가이드는 포트를 보존하는 $http_host). $scheme이라 TLS 연결 시
 * https 자동 전파.
 */
function proxyHeaders(hostVar: "$host" | "$http_host") {
  return [
    { name: "Host", value: hostVar },
    { name: "X-Real-IP", value: "$remote_addr" },
    { name: "X-Forwarded-For", value: "$proxy_add_x_forwarded_for" },
    { name: "X-Forwarded-Proto", value: "$scheme" },
  ];
}

const nexusHeaders = () => proxyHeaders("$host");
const gitlabHeaders = () => proxyHeaders("$http_host");

const LONG_TIMEOUTS = { read: "300s", send: "300s", connect: "" };
/** GitLab: 대용량 clone/push·CI 아티팩트 — 공식 외부-nginx 권장 3600s. */
const GITLAB_TIMEOUTS = { read: "3600s", send: "3600s", connect: "" };

export const SERVER_TEMPLATES: ServerTemplate[] = [
  {
    id: "nexus",
    name: "Nexus Repository (웹 UI + 저장소)",
    description:
      "Sonatype Nexus 3 웹 UI와 Maven/npm/PyPI 등 저장소 API(기본 포트 8081)를 리버스 프록시합니다. 대용량 아티팩트 업로드를 위해 본문 크기 1g·타임아웃 300s로 설정합니다.",
    tags: ["Nexus", "Maven/npm", "8081"],
    previewServerName: "nexus.example.com",
    defaults: {
      listen: 80,
      proxyPass: "http://127.0.0.1:8081",
      serverDirectives: ds({ bodySize: "1g" }),
      locationDirectives: ds({ headers: nexusHeaders(), timeouts: LONG_TIMEOUTS }),
    },
  },
  {
    id: "nexus-docker",
    name: "Nexus Docker 레지스트리 (별도 도메인)",
    description:
      "Nexus Docker 저장소의 HTTP 커넥터(예: 8082 — 저장소 설정의 커넥터 포트로 수정)를 전용 도메인으로 프록시합니다. docker push 레이어 업로드를 위해 본문 크기 무제한(0)·HTTP/1.1을 사용합니다.",
    tags: ["Nexus", "Docker", "8082"],
    previewServerName: "docker.example.com",
    defaults: {
      listen: 80,
      proxyPass: "http://127.0.0.1:8082",
      serverDirectives: ds({ bodySize: "0" }),
      locationDirectives: ds({ headers: nexusHeaders(), timeouts: LONG_TIMEOUTS, httpVersion: "1.1" }),
    },
  },
  {
    id: "nexus-combined",
    name: "Nexus 통합 (웹 + Docker, 한 도메인)",
    description:
      "한 도메인에서 /는 Nexus 웹 UI·저장소(8081)로, /v2/는 Docker 커넥터(예: 8082)로 보내는 location별 구성입니다. 본문 크기 무제한(0).",
    tags: ["Nexus", "Docker", "location별"],
    previewServerName: "nexus.example.com",
    defaults: {
      listen: 80,
      serverDirectives: ds({ bodySize: "0" }),
      locations: [
        {
          path: "/",
          mode: "direct",
          upstreamId: "",
          proxyPass: "http://127.0.0.1:8081",
          directives: ds({ headers: nexusHeaders(), timeouts: LONG_TIMEOUTS }),
        },
        {
          path: "/v2/",
          mode: "direct",
          upstreamId: "",
          proxyPass: "http://127.0.0.1:8082",
          directives: ds({ headers: nexusHeaders(), timeouts: LONG_TIMEOUTS, httpVersion: "1.1" }),
        },
      ],
    },
  },
  {
    id: "gitlab",
    name: "GitLab (웹 + Git over HTTP)",
    description:
      "GitLab 웹 UI와 git clone/push(HTTP)·LFS를 리버스 프록시합니다(placeholder 8181 — GitLab workhorse 또는 GitLab 컨테이너의 HTTP 포트로 수정). 대용량 저장소를 위해 본문 크기 무제한(0)·타임아웃 3600s, 포트 보존을 위해 Host $http_host를 사용합니다.",
    tags: ["GitLab", "Git/LFS", "8181"],
    previewServerName: "gitlab.example.com",
    defaults: {
      listen: 80,
      proxyPass: "http://127.0.0.1:8181",
      serverDirectives: ds({ bodySize: "0" }),
      locationDirectives: ds({ headers: gitlabHeaders(), timeouts: GITLAB_TIMEOUTS }),
    },
  },
  {
    id: "gitlab-registry",
    name: "GitLab Container Registry (별도 도메인)",
    description:
      "GitLab Container Registry(기본 포트 5050 — 인스턴스 설정에 맞게 수정)를 전용 도메인으로 프록시합니다. docker push 레이어 업로드를 위해 본문 크기 무제한(0)·HTTP/1.1·타임아웃 3600s를 사용합니다.",
    tags: ["GitLab", "Docker", "5050"],
    previewServerName: "registry.gitlab.example.com",
    defaults: {
      listen: 80,
      proxyPass: "http://127.0.0.1:5050",
      serverDirectives: ds({ bodySize: "0" }),
      locationDirectives: ds({ headers: gitlabHeaders(), timeouts: GITLAB_TIMEOUTS, httpVersion: "1.1" }),
    },
  },
];

export function getServerTemplate(id: string): ServerTemplate | undefined {
  return SERVER_TEMPLATES.find((t) => t.id === id);
}
