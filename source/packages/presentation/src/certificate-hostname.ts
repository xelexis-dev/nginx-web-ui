/**
 * ASCII DNS-name coverage for certificate warnings (RFC 9525 section 6.3).
 * A wildcard is permitted only as the certificate name's entire leftmost label
 * and matches exactly one hostname label. One trailing root dot is normalized.
 * This is not a TLS verifier: no certificate trust/expiry, Unicode/IDNA conversion,
 * IP-ID matching, or public-suffix policy is performed. Unsupported names fail closed.
 */
export function certificateCoversHostname(certificateName: string, hostname: string): boolean {
  if (typeof certificateName !== "string" || typeof hostname !== "string") return false;
  const wildcard = certificateName.startsWith("*.");
  const pattern = asciiDnsName(wildcard ? certificateName.slice(2) : certificateName);
  const name = asciiDnsName(hostname);
  if (!pattern || !name || (wildcard && pattern.length + 2 > 253)) return false;
  if (!wildcard) return pattern === name;
  const firstDot = name.indexOf(".");
  return firstDot > 0 && name.slice(firstDot + 1) === pattern;
}

function asciiDnsName(value: string): string | null {
  const raw = value.endsWith(".") ? value.slice(0, -1) : value;
  if (!raw || raw.length > 253 || !/^[A-Za-z0-9.-]+$/.test(raw) || /^[0-9.]+$/.test(raw)) return null;
  const name = raw.toLowerCase();
  if (!name.split(".").every(label => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label))) return null;
  return name;
}
