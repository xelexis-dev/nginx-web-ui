import {
  Table,
  Td,
  Th,
  Tr,
  Card,
  CardFoot,
  Block,
  FormStack,
  FormBar,
  Checkbox,
  Field,
  FieldGroup,
  SegmentedControl,
  Switch,
  TextInput,
} from "@repo/ui";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

it("supports a header-only category and a footer outside its body", () => {
  const headerOnly = renderToStaticMarkup(
    <Card id="password" title="비밀번호" />,
  );
  expect(headerOnly).not.toMatch(/class="[^"]*cardBody/);
  const withFoot = renderToStaticMarkup(
    <Card id="keys" title="패스키" footer={<CardFoot>최대 10개</CardFoot>}>
      <p>목록</p>
    </Card>,
  );
  expect(withFoot).toMatch(/<p>목록<\/p><\/div><div class="[^"]*cardFoot/);
  expect(withFoot).toContain("최대 10개");
});

// The shared form family that page forms now use instead of their own label / hint / error markup:
// what reaches the DOM (ids, descriptions, native attributes), not how it looks.

function attrsOf(html: string, tag: string, marker: string) {
  const el = [...html.matchAll(new RegExp(`<${tag}\\b[^>]*>`, "g"))]
    .map((m) => m[0])
    .find((m) => m.includes(marker));
  if (!el) throw new Error(`no <${tag}> with ${marker} in ${html}`);
  return Object.fromEntries(
    [...el.matchAll(/([\w-]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]),
  );
}

describe("FieldGroup", () => {
  it("names its controls with a legend and describes the whole group with its hint and error", () => {
    const html = renderToStaticMarkup(
      <FieldGroup
        id="challenge"
        legend="검증 방식"
        hint="DNS는 TXT 레코드가 필요합니다"
        error="고르세요"
        required
        wide
      >
        <SegmentedControl
          label="검증 방식"
          value="http-01"
          onChange={() => {}}
          options={[{ value: "http-01", label: "HTTP" }]}
        />
      </FieldGroup>,
    );
    const set = attrsOf(html, "fieldset", 'id="challenge"');
    expect(set["aria-describedby"]?.split(" ")).toEqual([
      "challenge-hint",
      "challenge-error",
    ]);
    expect(set["data-wide"]).toBe("");
    expect(html).toMatch(/<legend[^>]*>검증 방식<\/legend>/);
    expect(html).not.toContain("필수");
    expect(attrsOf(html, "div", 'id="challenge-hint"')).toBeDefined();
    expect(attrsOf(html, "p", 'id="challenge-error"')).toBeDefined();
  });

  it("has no description when there is neither hint nor error", () => {
    const html = renderToStaticMarkup(
      <FieldGroup id="g" legend="받을 사건">
        <span>x</span>
      </FieldGroup>,
    );
    expect(
      attrsOf(html, "fieldset", 'id="g"')["aria-describedby"],
    ).toBeUndefined();
    expect(html).not.toContain("g-hint");
  });
});

describe("Checkbox", () => {
  it("passes the native form attributes through and keeps type checkbox first", () => {
    const html = renderToStaticMarkup(
      <Checkbox
        name="events"
        value="certificate.expiring"
        defaultChecked
        required
        disabled
        form="f"
        label="인증서 만료 예정"
      />,
    );
    const input = attrsOf(html, "input", 'name="events"');
    expect(input.type).toBe("checkbox");
    expect(input.value).toBe("certificate.expiring");
    expect(input.form).toBe("f");
    // type comes first (the existing 2FA test matches `type="checkbox"[^>]*required=""`); React writes
    // `checked` after the other attributes of an input, so the rest are checked one by one.
    expect(html).toMatch(/<input type="checkbox"/);
    for (const attr of ['checked=""', 'required=""', 'disabled=""'])
      expect(html).toMatch(new RegExp(`<input type="checkbox"[^>]*${attr}`));
    expect(html).toMatch(/<label[^>]*>.*인증서 만료 예정.*<\/label>/);
  });

  it("links its description and keeps a description the caller passed", () => {
    const html = renderToStaticMarkup(
      <Checkbox
        id="ack"
        aria-describedby="outside"
        label="확인했습니다"
        description="이전 코드는 모두 무효가 됩니다"
      />,
    );
    expect(
      attrsOf(html, "input", 'id="ack"')["aria-describedby"]?.split(" "),
    ).toEqual(["outside", "ack-description"]);
    expect(attrsOf(html, "span", 'id="ack-description"')).toBeDefined();
  });

  it("puts a caller className and style on the row, not on the box", () => {
    const html = renderToStaticMarkup(
      <Checkbox className="toolbar" style={{ padding: 4 }} label="숨김" />,
    );
    const row = attrsOf(html, "label", "toolbar");
    expect(row.class).toContain("toolbar");
    expect(row.style).toBe("padding:4px");
    const box = attrsOf(html, "input", 'type="checkbox"');
    expect(box.class).not.toContain("toolbar");
    expect(box.style).toBeUndefined();
  });
});

describe("Field (the contract the migration relies on)", () => {
  it("takes a hint of several paragraphs as one description", () => {
    const html = renderToStaticMarkup(
      <Field
        label="제외 도메인"
        htmlFor="hosts"
        hint={
          <>
            <p>한 줄에 하나씩.</p>
            <p>적용은 지금부터입니다.</p>
          </>
        }
      >
        <TextInput name="hosts" />
      </Field>,
    );
    expect(html).toMatch(
      /<div[^>]*id="hosts-hint"[^>]*><p>한 줄에 하나씩\.<\/p><p>적용은 지금부터입니다\.<\/p><\/div>/,
    );
    expect(attrsOf(html, "input", 'id="hosts"')["aria-describedby"]).toBe(
      "hosts-hint",
    );
  });

  it("derives the hint and error ids from htmlFor and marks the control", () => {
    const html = renderToStaticMarkup(
      <Field
        label="이름"
        htmlFor="name"
        hint="영문"
        error="필요합니다"
        required
      >
        <TextInput name="name" defaultValue="a" />
      </Field>,
    );
    const input = attrsOf(html, "input", 'id="name"');
    expect(input.name).toBe("name");
    expect(input["aria-describedby"]?.split(" ")).toEqual([
      "name-hint",
      "name-error",
    ]);
    expect(input["aria-invalid"]).toBe("true");
    expect(input["aria-required"]).toBe("true");
  });
});

describe("form layout options", () => {
  it("keeps the existing layout by default and adds an optional group grid", () => {
    const plain = renderToStaticMarkup(
      <FieldGroup id="plain" legend="대상">
        <TextInput name="plain" />
      </FieldGroup>,
    );
    expect(plain).not.toContain("data-columns");
    const grid = renderToStaticMarkup(
      <FieldGroup id="grid" legend="대상" columns={2} hint="서로 다른 두 대상">
        <Field label="첫 대상" htmlFor="first">
          <TextInput name="first" />
        </Field>
        <Field label="둘째 대상" htmlFor="second">
          <TextInput name="second" />
        </Field>
      </FieldGroup>,
    );
    expect(grid).toContain('data-columns="2"');
    expect(grid).toContain('for="first"');
    expect(grid).toContain('for="second"');
    expect(attrsOf(grid, "fieldset", 'id="grid"')["aria-describedby"]).toBe(
      "grid-hint",
    );
  });

  it("keeps inline controls labelled, disabled and connected to all descriptions", () => {
    const html = renderToStaticMarkup(
      <Field
        layout="inline"
        label="자동 갱신"
        htmlFor="renew"
        hint="매일 검사"
        error="설정을 확인하세요"
        describedBy="policy"
        required
      >
        <TextInput name="renew" disabled aria-describedby="outside" />
      </Field>,
    );
    expect(html).toContain('data-layout="inline"');
    expect(html).toContain('for="renew"');
    const input = attrsOf(html, "input", 'id="renew"');
    expect(input.disabled).toBe("");
    expect(input.name).toBe("renew");
    expect(input["aria-required"]).toBe("true");
    expect(input["aria-invalid"]).toBe("true");
    expect(input["aria-describedby"]?.split(" ")).toEqual([
      "outside",
      "policy",
      "renew-hint",
      "renew-error",
    ]);
    expect(html).not.toContain("필수");
  });

  it("keeps render-prop fields connected in the inline layout", () => {
    const html = renderToStaticMarkup(
      <Field layout="inline" label="갱신" htmlFor="toggle" hint="만료 전 검사">
        {(control) => (
          <button {...control} type="button" role="switch" aria-checked="false">
            갱신
          </button>
        )}
      </Field>,
    );
    const button = attrsOf(html, "button", 'id="toggle"');
    expect(button["aria-describedby"]).toBe("toggle-hint");
    expect(button.type).toBe("button");
  });

  it("keeps the Card anchor, heading and field content", () => {
    const html = renderToStaticMarkup(
      <Card id="origin" title="원본" headingLevel={3}>
        <TextInput name="origin" />
      </Card>,
    );
    expect(html).toContain('aria-labelledby="origin-heading"');
    expect(html).toMatch(
      /<h3[^>]*id="origin-heading"[^>]*tabindex="-1">원본<\/h3>/,
    );
    expect(html).toContain('name="origin"');
  });
});

describe("category card and repeated block", () => {
  it("hides an off card body while keeping its heading and switch", () => {
    const html = renderToStaticMarkup(
      <Card
        id="tls"
        title="TLS"
        off
        actions={
          <button role="switch" aria-checked="false">
            TLS 사용
          </button>
        }
      >
        <input name="certificate" />
      </Card>,
    );
    expect(html).toContain('aria-labelledby="tls-heading"');
    expect(html).toContain('hidden=""');
    expect(html).toContain('name="certificate"');
    expect(html).toContain('role="switch"');
  });
  it("renders the error count and a loading status without dropping the heading", () => {
    const html = renderToStaticMarkup(
      <Card id="server" title="서버" loading errorCount={2}>
        <input name="server" />
      </Card>,
    );
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('data-error=""');
    expect(html).toContain("오류 2");
    expect(html).toContain('role="status"');
    expect(html).not.toContain('name="server"');
    expect(html).toContain('id="server-heading"');
  });
  it("keeps a collapsed block mounted and connects its body to the caller's control", () => {
    const html = renderToStaticMarkup(
      <Block
        id="route"
        number={1}
        title="/api"
        expanded={false}
        errorCount={1}
        actions={
          <button aria-controls="route-body" aria-expanded="false">
            펼치기
          </button>
        }
      >
        <input name="path" />
      </Block>,
    );
    expect(html).toContain('aria-labelledby="route-heading"');
    expect(html).toMatch(/id="route-body"[^>]*hidden=""/);
    expect(html).toContain('name="path"');
    expect(html).toContain('aria-controls="route-body"');
    expect(html).toContain("오류 1");
  });
  it("keeps disabled inert fieldsets and a single native form submission", () => {
    const html = renderToStaticMarkup(
      <form>
        <fieldset disabled inert>
          <FormStack>
            <Card id="server" title="서버">
              <Field label="이름" htmlFor="disabled-name" required span={8}>
                <TextInput name="name" />
              </Field>
            </Card>
            <FormBar note="변경 1건">
              <button type="submit">저장</button>
            </FormBar>
          </FormStack>
        </fieldset>
      </form>,
    );
    expect(html.match(/<form[ >]/g)).toHaveLength(1);
    expect(html).toMatch(/<fieldset disabled="" inert=""/);
    expect(html).toContain('data-span="8"');
    expect(html).toContain('aria-required="true"');
    expect(html).not.toContain("필수");
  });
  it("shows optional only when the field is not required", () => {
    const optional = renderToStaticMarkup(
      <Field label="메모" htmlFor="memo" optional span={12}>
        <TextInput name="memo" />
      </Field>,
    );
    expect(optional).toContain("(선택)");
    expect(optional).toContain('data-span="12"');
    const required = renderToStaticMarkup(
      <Field label="메모" htmlFor="memo" optional required>
        <TextInput />
      </Field>,
    );
    expect(required).not.toContain("(선택)");
    expect(required).toContain('aria-required="true"');
  });
});

describe("table density", () => {
  it.each([undefined, "comfortable"] as const)(
    "preserves one semantic table and the current row (%s)",
    (density) => {
      const html = renderToStaticMarkup(
        <Table label="노드" density={density}>
          <thead>
            <tr>
              <Th>이름</Th>
            </tr>
          </thead>
          <tbody>
            <Tr current>
              <Td cell="primary">
                <a href="/nodes/one">서울</a>
              </Td>
            </Tr>
          </tbody>
        </Table>,
      );
      expect(html).toContain(`data-density="${density ?? "dense"}"`);
      expect(html.match(/<table /g)).toHaveLength(1);
      expect(html.match(/href="\/nodes\/one"/g)).toHaveLength(1);
      expect(html).toContain('scope="col"');
      expect(html).toContain('data-current=""');
    },
  );
});

// The visible word is fixed; aria-checked conveys the value without changing the submit contract.
it("Switch keeps its visible word and accessible state on either side of a toggle", () => {
  for (const checked of [false, true]) {
    const change = vi.fn();
    const props = {
      checked,
      label: "메일 발송",
      visibleLabel: "사용",
      onCheckedChange: change,
    };
    const html = renderToStaticMarkup(<Switch {...props} />);
    expect(html).toContain('role="switch"');
    expect(html).toContain(`aria-checked="${checked}"`);
    expect(html).toContain('aria-label="메일 발송"');
    expect(html).toMatch(/<span[^>]*>사용<\/span>/);
    expect(html).not.toMatch(/>비?활성<\/span>/);
    const button = Switch(props);
    button.props.onClick({ defaultPrevented: false });
    expect(change).toHaveBeenCalledExactlyOnceWith(!checked);
  }
});

vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: (_action: unknown, initial: unknown) => [
    initial,
    () => {},
    false,
  ],
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async (previous: unknown) => previous,
}));
vi.mock("@/lib/api-page", () => ({
  readPageApi: async () => ({
    id: "sso",
    name: "SSO",
    issuer: "https://id.example.test",
    clientId: "client",
    scope: "openid",
    enabled: true,
    sortOrder: 0,
  }),
}));
import { SmtpForm } from "@/app/admin/settings/smtp/smtp-form";
import { SecurityForm } from "@/app/admin/security/security-form";
import { MailRecipientForm } from "@/app/admin/notifications/mail/recipient-form";
import { RecipientListView } from "@/app/admin/notifications/mail/recipient-list";
import { WebhookForm } from "@/app/admin/notifications/webhooks/webhook-form";
import { ProviderForm } from "@/app/admin/oauth/provider-form";
import { TwoFactorEnroll } from "@/app/(app)/account/2fa/two-factor-forms";
import EditOAuthProviderPage from "@/app/admin/oauth/[id]/edit/page";
const smtpDefaults = {
  enabled: true,
  host: "smtp.example.test",
  port: 587,
  secure: false,
  username: "ops",
  fromEmail: "ops@example.test",
  fromName: "Ops",
  rejectUnauthorized: true,
};

it.each([false, true])(
  "boolean settings keep existing hidden submit values: %s",
  (enabled) => {
    const forms = [
      renderToStaticMarkup(
        <SmtpForm defaults={{ ...smtpDefaults, enabled }} hasStoredPassword />,
      ),
      renderToStaticMarkup(
        <MailRecipientForm
          mode="create"
          defaults={{
            address: "ops@example.test",
            label: "Ops",
            enabled,
            events: ["cert.issued"],
          }}
          smtpReady={false}
        />,
      ),
      renderToStaticMarkup(
        <WebhookForm
          mode="create"
          defaults={{
            name: "Ops",
            enabled,
            format: "generic",
            events: ["cert.issued"],
            bodyTemplate: "",
          }}
        />,
      ),
      renderToStaticMarkup(
        <ProviderForm
          mode="edit"
          defaults={{
            id: "sso",
            name: "SSO",
            issuer: "https://id.example.test",
            clientId: "client",
            scope: "openid",
            enabled,
            sortOrder: 0,
          }}
        />,
      ),
    ];
    for (const html of forms) {
      expect(attrsOf(html, "input", 'name="enabled"')).toMatchObject({
        type: "hidden",
        value: String(enabled),
      });
      expect(html).toMatch(
        new RegExp(`role="switch"[^>]*aria-checked="${enabled}"`),
      );
    }
    expect(forms[0]).toContain('name="password" value="__keep__"');
    const policy = renderToStaticMarkup(
      <SecurityForm
        requireAdmin={enabled}
        requireUser={enabled}
        requireOauth={enabled}
      />,
    );
    for (const name of ["require2faAdmin", "require2faUser", "require2faOauth"])
      expect(attrsOf(policy, "input", `name="${name}"`)).toMatchObject({
        type: "hidden",
        value: String(enabled),
      });
    expect(policy.match(/>강제<\/span>/g)).toHaveLength(3);
  },
);
it("SMTP has only secondary CardFoot submits in Save then Test order", () => {
  const html = renderToStaticMarkup(
    <SmtpForm defaults={smtpDefaults} hasStoredPassword />,
  );
  expect(html).not.toContain("nwui-btn-primary");
  expect(html).not.toMatch(/class="[^"]*formBar/);
  expect(html).toMatch(
    /cardFoot[^]*>저장<\/button>[^]*>연결 확인 \/ 테스트 발송<\/button>/,
  );
});
it("form exits use approved destinations and OAuth edit has no header action", async () => {
  const enroll = renderToStaticMarkup(
    <TwoFactorEnroll secret="fixture" qrDataUrl="data:image/png;base64," />,
  );
  expect(enroll).toMatch(/href="\/account"[^>]*>취소<\/a>/);
  expect(enroll.indexOf(">취소</a>")).toBeLessThan(
    enroll.indexOf("2FA 활성화</button>"),
  );
  const provider = renderToStaticMarkup(
    <ProviderForm
      mode="edit"
      defaults={{
        id: "sso",
        name: "SSO",
        issuer: "https://id.example.test",
        clientId: "client",
        scope: "openid",
        enabled: true,
        sortOrder: 0,
      }}
    />,
  );
  expect(provider).toMatch(/href="\/admin\/oauth\/sso"[^>]*>취소<\/a>/);
  expect(provider).toContain("끄면 설정만 보관합니다.");
  expect(
    renderToStaticMarkup(
      await EditOAuthProviderPage({ params: Promise.resolve({ id: "sso" }) }),
    ),
  ).not.toContain("상세로");
});
it("SMTP-not-ready notices announce an alert in recipient list and form", () => {
  for (const node of [
    <RecipientListView
      key="list"
      recipients={[]}
      mailer={{
        ok: false,
        reason: "disabled",
        message: "SMTP가 꺼져 있습니다.",
      }}
    />,
    <MailRecipientForm
      key="form"
      mode="create"
      defaults={{ address: "", label: "", enabled: true, events: [] }}
      smtpReady={false}
    />,
  ])
    expect(renderToStaticMarkup(node)).toMatch(
      /data-tone="error"[^>]*role="alert"/,
    );
});
