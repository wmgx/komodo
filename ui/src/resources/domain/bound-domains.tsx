import { komodo_client, useRead } from "@/lib/hooks";
import { Alert, Checkbox, Stack, Text } from "@mantine/core";
import { Types } from "komodo_client";

/** Domains bound to a Stack / Deployment. Empty for other resources. */
export function useBoundDomains(type: string, id: string) {
  const supported = type === "Stack" || type === "Deployment";
  const { data } = useRead(
    "ListServiceDomains",
    { target: { type, id } as Types.ResourceTarget },
    { enabled: supported },
  );
  return supported ? (data ?? []) : [];
}

/** Deletes the Domains (remote cleanup first). Throws on the first failure. */
export async function deleteDomains(domains: Types.DomainListItem[]) {
  for (const d of domains) {
    await komodo_client().write("DeleteDomain", { id: d.id });
  }
}

export function BoundDomainsNotice({
  domains,
  checked,
  onChange,
}: {
  domains: Types.DomainListItem[];
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  if (!domains.length) return null;
  return (
    <Alert color="yellow" title="该服务绑定了域名">
      <Stack gap="xs">
        <Text fz="sm">{domains.map((d) => d.info.hostname).join(", ")}</Text>
        <Checkbox
          checked={checked}
          onChange={(e) => onChange(e.currentTarget.checked)}
          label="同时删除这些域名（清理 Cloudflare 上的记录）"
        />
        {!checked && (
          <Text fz="xs" c="dimmed">
            不勾选则保留域名，其状态会显示为「目标服务已不存在」。
          </Text>
        )}
      </Stack>
    </Alert>
  );
}
