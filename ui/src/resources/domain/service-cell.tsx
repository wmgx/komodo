import { useListItemQuery } from "@/lib/hooks";
import ResourceLink from "@/resources/link";
import { domainStatusIntention } from "@/lib/color";
import { Group, Text } from "@mantine/core";
import { StatusBadge } from "mogh_ui";
import { Types } from "komodo_client";
import { TARGET_GONE } from "./text";

/** True once we know the bound Stack / Deployment no longer exists. */
export function useTargetGone(target: Types.ResourceTarget | undefined) {
  const supported =
    target?.type === "Stack" || target?.type === "Deployment";
  const q = useListItemQuery(
    (supported ? target.type : "Stack") as "Stack",
    supported ? target.id : undefined,
  );
  return (
    supported &&
    q.isSuccess &&
    !(q.data as { id: string }[]).some((r) => r.id === target.id)
  );
}

/** Status badge which says so when the bound service is gone. */
export function StatusCell({ item }: { item: Types.DomainListItem }) {
  const gone = useTargetGone(item.info.target);
  const code = gone ? "error" : item.info.status_code;
  return (
    <StatusBadge
      text={gone ? TARGET_GONE : item.info.status_code}
      intent={domainStatusIntention(code)}
    />
  );
}

/** Clickable bound service, or a note that it is gone. */
export default function ServiceCell({
  target,
  service,
}: {
  target?: Types.ResourceTarget;
  service?: string;
}) {
  const gone = useTargetGone(target);
  if (!target || (target.type !== "Stack" && target.type !== "Deployment"))
    return <Text c="dimmed">-</Text>;
  if (gone) return <Text c="red">{TARGET_GONE}</Text>;
  return (
    <Group gap={4}>
      <ResourceLink type={target.type} id={target.id} />
      {service && <Text c="dimmed">/ {service}</Text>}
    </Group>
  );
}
