export type BrowserAddress = Pick<
  URL,
  "protocol" | "host" | "hostname" | "origin"
>;
export type Remedy = "D" | "E" | "F" | "G" | "UX" | "U0";
export interface PasskeyNotice {
  title: string;
  lead: string;
  rows: { label: string; value: string; tone?: "success" | "warning" }[];
  cause: string;
  remedy: Remedy | null;
  remedyLabel: string;
  items: string[];
  footer: boolean;
  pending: boolean;
}

export function selectPasskeyNotice(
  reason: string | null,
  browser: BrowserAddress | null,
  role: string,
): PasskeyNotice {
  const admin = role === "admin";
  const rows: PasskeyNotice["rows"] = [];
  let cause = "서버가 이 접속에서는 패스키를 쓸 수 없다고 알렸습니다.";
  let remedy: Remedy | null = "U0";
  if (reason === "insecure_origin") {
    rows.push({
      label: "서버가 확인한 연결",
      value: "패스키 접속 조건 확인 실패",
      tone: "warning",
    });
    cause = "서버가 이 접속을 HTTPS로 확인하지 못했습니다.";
    remedy = null;
    if (browser) {
      rows.push({
        label: "브라우저 연결",
        value: `${browser.protocol === "https:" ? "HTTPS" : "HTTP"} ${browser.origin}`,
        tone: browser.protocol === "https:" ? "success" : "warning",
      });
      if (browser.protocol === "https:") {
        cause +=
          " CDN이나 로드 밸런서가 앞에서 TLS를 끝내고 서버에는 HTTP로 넘기는 구성이 흔한 원인입니다.";
        remedy = "D";
      } else if (
        browser.protocol === "http:" &&
        browser.hostname !== "localhost"
      ) {
        cause +=
          " 지금 주소가 HTTP이기 때문입니다. 패스키는 HTTPS 주소에서만 동작합니다.";
        remedy = "E";
      } else {
        cause +=
          " 브라우저 주소와 서버가 확인한 주소가 다릅니다. 서버 앞의 프록시가 원래 주소를 그대로 넘기지 않는 구성일 수 있습니다.";
        remedy = "UX";
      }
    }
  } else if (reason === "ip_host") {
    rows.push({
      label: "서버가 확인한 주소",
      value: "IP 주소",
      tone: "warning",
    });
    cause =
      "서버가 확인한 접속 주소가 IP 주소입니다. 패스키는 도메인 이름에 묶이기 때문에 IP 주소에서는 동작하지 않습니다.";
    remedy = null;
    if (browser) {
      rows.push({ label: "브라우저 주소", value: browser.host });
      remedy =
        /^\d{1,3}(\.\d{1,3}){3}$/.test(browser.hostname) ||
        browser.hostname.startsWith("[")
          ? "F"
          : "UX";
      if (remedy === "UX")
        cause +=
          " 브라우저 주소와 서버가 확인한 주소가 다릅니다. 서버 앞의 프록시가 원래 주소를 그대로 넘기지 않는 구성일 수 있습니다.";
    }
  } else if (reason === "auth_url_http") {
    rows.push({
      label: admin ? "관리 주소 설정" : "서버의 관리 주소",
      value: admin ? "HTTP 주소 AUTH_URL" : "HTTP 주소",
      tone: "warning",
    });
    cause = `${admin ? "관리 주소 설정 AUTH_URL이" : "서버의 관리 주소 설정이"} HTTP 주소입니다. 서버는 이 설정을 기준으로 판단하므로, 브라우저에서 HTTPS로 접속해도 패스키를 쓸 수 없습니다.`;
    remedy = "G";
  }
  const fixes: Record<Remedy, string[]> = admin
    ? {
        D: [
          "이 서버에서 TLS를 끝냅니다. 이 제품의 리버스 프록시가 443 포트와 인증서로 관리 화면(대상 http://127.0.0.1:8080)을 제공하게 합니다. CDN을 쓴다면 CDN과 이 서버 사이도 HTTPS로 설정합니다(예: 「Full (strict)」 모드).",
          "TLS를 앞에서 끝내야 한다면 AUTH_URL을 지금 쓰는 HTTPS 주소로 설정하고 컨테이너를 다시 시작합니다. 그 뒤에는 그 주소로만 로그인할 수 있습니다.",
        ],
        E: [
          "이 제품의 리버스 프록시로 HTTPS 도메인을 만들고, 관리 화면을 그 주소로 엽니다.",
          "설치한 컴퓨터에서는 http://localhost로 열어도 패스키를 쓸 수 있습니다.",
        ],
        F: [
          "IP 주소 대신 HTTPS 도메인 이름으로 관리 화면을 엽니다. 도메인이 아직 없다면 이 제품의 리버스 프록시에 도메인과 인증서를 설정합니다.",
        ],
        G: ["AUTH_URL을 HTTPS 주소로 바꾸고 컨테이너를 다시 시작합니다."],
        UX: [
          "브라우저에서 쓰는 주소가 그대로 서버까지 오도록, 앞단 프록시가 이 제품의 리버스 프록시(443 포트와 인증서)로 원래 주소를 넘기게 합니다.",
          "또는 AUTH_URL을 실제로 접속하는 HTTPS 주소로 설정하고 컨테이너를 다시 시작합니다.",
        ],
        U0: [
          "설치 안내의 「패스키로 로그인」에서 패스키를 쓸 수 있는 접속 조건을 확인하세요.",
        ],
      }
    : {
        D: [
          "서버 앞단의 설정을 바꿔야 해서 이 설치를 운영하는 관리자만 해결할 수 있습니다. 관리자에게 이 내용을 알려 주세요.",
        ],
        E: [
          "이 서버의 HTTPS 주소를 알고 있다면 그 주소로 접속하세요. 모르면 관리자에게 문의하세요.",
        ],
        F: [
          "이 서버의 도메인 주소를 알고 있다면 그 주소로 접속하세요. 모르면 관리자에게 문의하세요.",
        ],
        G: [
          "이 설정은 이 설치를 운영하는 관리자만 바꿀 수 있습니다. 관리자에게 이 내용을 알려 주세요.",
        ],
        UX: ["관리자에게 이 내용을 알려 주세요."],
        U0: ["관리자에게 이 내용을 알려 주세요."],
      };
  const items = remedy ? fixes[remedy] : [];
  return {
    title: "이 접속에서는 패스키를 사용할 수 없습니다",
    lead: "패스키 등록과 패스키 로그인만 꺼집니다. 다른 로그인 방법은 그대로 쓸 수 있습니다.",
    rows,
    cause,
    remedy,
    remedyLabel: admin
      ? items.length > 1
        ? "해결 방법 · 둘 중 하나"
        : "해결 방법"
      : "할 수 있는 일",
    items,
    footer: admin && remedy !== "U0" && remedy !== null,
    pending: remedy === null,
  };
}
