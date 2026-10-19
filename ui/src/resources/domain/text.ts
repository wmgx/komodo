import { Types } from "komodo_client";

/** Plain language for a machine readable domain problem. */
export function reasonText(r: Types.DomainReason): string {
  switch (r.code) {
    case "AccountNotFound":
      return "The Cloudflare account was deleted. Check Settings -> Domains.";
    case "ZoneNotFound":
      return "The account has no matching domain. Check the name and the token permissions.";
    case "TunnelNotFound":
      return "No public connection for this server. Make a service public to set one up.";
    case "TunnelAmbiguous":
      return "This server is linked to several tunnels. Keep one in the account settings.";
    case "AuthFailed":
      return "The Cloudflare token is invalid or lacks permissions. Update it in Settings -> Domains.";
    case "RateLimited":
      return "Cloudflare is rate limiting requests. Try again in a moment.";
    case "ProviderUnavailable":
    case "Timeout":
      return "Cloudflare could not be reached. Try again in a moment.";
    case "RecordMissing":
      return "The DNS record was removed. Make the service public again to restore it.";
    case "RecordChanged":
      return "The DNS record was edited by hand. Applying the domain restores it.";
    case "RuleMissing":
      return "The route was removed from Cloudflare. Applying the domain restores it.";
    case "RuleChanged":
      return "The route was edited by hand. Applying the domain restores it.";
    case "ForeignRecordConflict":
      return "A record with this name already exists and is not managed by Komodo.";
    case "PlanChanged":
      return "Cloudflare changed since the preview. Preview again.";
    default:
      return r.message;
  }
}

export function statusReasons(
  status: Types.DomainStatus | undefined,
): Types.DomainReason[] {
  if (status?.type === "Error") return [status.data.reason];
  if (status?.type === "Drifted") return status.data.reasons;
  return [];
}

export const TARGET_GONE = "Service no longer exists";

/** Valid subdomain label, derived from any name. Same rule as the backend. */
export function toLabel(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63);
}

/** Returns an error message, or null when the subdomain is fine. */
export function validatePrefix(prefix: string): string | null {
  if (!prefix) return null;
  if (!/^[a-zA-Z0-9-]+$/.test(prefix))
    return "Use letters, digits and - only";
  if (prefix.startsWith("-") || prefix.endsWith("-"))
    return "Cannot start or end with -";
  if (prefix.length > 63) return "At most 63 characters";
  return null;
}
