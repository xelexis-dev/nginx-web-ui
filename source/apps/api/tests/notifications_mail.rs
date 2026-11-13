use nwui_api::{
    notifications::mail::*,
    notify::{RenderContext, sample_event},
};

#[test]
fn renewal_message_keeps_plaintext_instructions_and_escaped_html_alternative() {
    let mut event = sample_event(1_800_000_000_000);
    event.title = "<script> & \"title\"".into();
    event.message = Some("<img src=x onerror=alert(1)>".into());
    let content = mail_content(
        &event,
        RenderContext {
            base_url: Some("https://admin.example.test"),
        },
    );
    assert!(content.subject.contains("인증서 갱신 필요: example.com"));
    assert!(content.text.contains("TXT 레코드"));
    assert!(
        content
            .text
            .contains("https://admin.example.test/certificates/")
    );
    assert!(
        content
            .html
            .contains("&lt;script&gt; &amp; &quot;title&quot;")
    );
    assert!(!content.html.contains("<script>"));
    assert!(!content.html.contains("<img src=x"));
    assert!(content.html.contains("prefers-color-scheme: dark"));
    assert!(content.html.contains("UTC"));
}

#[test]
fn relative_and_unsafe_links_never_become_mail_buttons() {
    let mut event = sample_event(0);
    event.event = "cert.issued".into();
    let content = mail_content(&event, RenderContext::default());
    assert!(!content.html.contains("href="));
    assert!(!content.text.contains("바로가기"));
    event.path = Some("javascript:alert(1)".into());
    let content = mail_content(&event, RenderContext::default());
    assert!(!content.html.contains("href="));
    assert!(!content.text.contains("javascript:"));
}
