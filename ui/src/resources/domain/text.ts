import { Types } from "komodo_client";

/** Plain language for a machine readable domain problem. */
export function reasonText(r: Types.DomainReason): string {
  switch (r.code) {
    case "AccountNotFound":
      return "域名账号不存在或已被删除，请到 Settings -> Domains 检查。";
    case "ZoneNotFound":
      return "账号下找不到该主机名所在的域名区域（zone），请检查域名后缀或 token 权限。";
    case "TunnelNotFound":
      return "找不到可用的隧道。想从外网访问，请先给这台服务器开通隧道。";
    case "TunnelAmbiguous":
      return "这台服务器绑定了多条隧道，无法自动选择，请在账号设置里只保留一条。";
    case "AuthFailed":
      return "服务商 token 无效或权限不足，请到 Settings -> Domains 更新。";
    case "RateLimited":
      return "服务商限流了，请稍后重试。";
    case "ProviderUnavailable":
    case "Timeout":
      return "服务商暂时无法访问或超时，请稍后重试。";
    case "RecordMissing":
      return "DNS 记录被人为删除了，重新应用即可恢复。";
    case "RecordChanged":
      return "DNS 记录被手动改过，重新应用会恢复为 Komodo 的配置。";
    case "RuleMissing":
      return "隧道里的转发规则被删除了，重新应用即可恢复。";
    case "RuleChanged":
      return "隧道里的转发规则被手动改过，重新应用会恢复。";
    case "ForeignRecordConflict":
      return "远端已有同名记录，且不归 Komodo 管理，确认后才会覆盖。";
    case "PlanChanged":
      return "远端内容在预览后发生了变化，请重新预览。";
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

export const TARGET_GONE = "目标服务已不存在";

/** Returns an error message, or null when the hostname prefix is fine. */
export function validatePrefix(prefix: string): string | null {
  if (!prefix) return null;
  for (const label of prefix.split(".")) {
    if (!label) return "不能有空的部分（连续的点）";
    if (label.includes("_"))
      return `不能包含下划线，请用连字符 - 代替（例如 ${label.replaceAll("_", "-")}）`;
    if (!/^[a-zA-Z0-9-]+$/.test(label))
      return "只允许字母、数字和连字符 -";
    if (label.startsWith("-") || label.endsWith("-"))
      return "不能以连字符开头或结尾";
    if (label.length > 63) return "每一段最长 63 个字符";
  }
  return null;
}

export function isPrivateHost(host: string): boolean {
  const m = host.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (m) {
    const [a, b] = [Number(m[1]), Number(m[2])];
    return (
      a === 10 ||
      (a === 172 && b >= 16 && b < 32) ||
      (a === 192 && b === 168) ||
      (a === 169 && b === 254) ||
      (a === 100 && b >= 64 && b < 128)
    );
  }
  const h = host.toLowerCase();
  return h.startsWith("fc") || h.startsWith("fd") || h.startsWith("fe80");
}

export function domainLink(config: Types.DomainConfig): string {
  const tunnel = !!config.ingress?.account;
  return tunnel || !config.port
    ? `https://${config.hostname}`
    : `http://${config.hostname}:${config.port}`;
}
