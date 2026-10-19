import { useRead, useUser } from "@/lib/hooks";
import { domainStatusIntention } from "@/lib/color";
import { ICONS } from "@/lib/icons";
import ResourceLink from "@/resources/link";
import { Anchor, Collapse, Group, Stack, Text } from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { Section, StatusBadge } from "mogh_ui";
import { Types } from "komodo_client";
import { ServiceAccessButton } from "./make-public";
import { ServiceTarget } from "./service-hooks";
import { reasonText, statusReasons } from "./text";

/** Domain resource, mode and problems. For troubleshooting only. */
function Details({ item }: { item: Types.ServiceItem }) {
  const [opened, { toggle }] = useDisclosure();
  return (
    <>
      <Anchor component="button" type="button" fz="xs" c="dimmed" onClick={toggle}>
        {opened ? "Hide details" : "Details"}
      </Anchor>
      <Collapse expanded={opened}>
        <Stack gap={4}>
          {item.public.map((p) => (
            <DetailRow key={p.domain_id} url={p} />
          ))}
        </Stack>
      </Collapse>
    </>
  );
}

function DetailRow({ url }: { url: Types.ServicePublicUrl }) {
  const reasons = statusReasons(
    useRead("GetDomain", { domain: url.domain_id }).data?.info?.status,
  );
  return (
    <Stack gap={2}>
      <Group gap="xs">
        <ResourceLink type="Domain" id={url.domain_id} />
        <Text fz="xs" c="dimmed">
          {url.url.startsWith("https") ? "Via tunnel" : "Via DNS"}
        </Text>
      </Group>
      {reasons.map((r, i) => (
        <Text key={i} fz="xs" c="dimmed">
          {reasonText(r)}
        </Text>
      ))}
    </Stack>
  );
}

function Row({ item, showName }: { item: Types.ServiceItem; showName: boolean }) {
  return (
    <Group justify="space-between" align="flex-start" wrap="nowrap">
      <Stack gap={4}>
        {showName && <Text fw="bold">{item.name}</Text>}
        {item.ports.length ? (
          item.ports.map((p) => (
            <Group key={p.host} gap="xs">
              <Text fz="sm" c="dimmed">
                {p.host} {"->"} {p.container}
              </Text>
              {p.lan_url && (
                <Anchor href={p.lan_url} target="_blank" rel="noreferrer" fz="sm">
                  {p.lan_url}
                </Anchor>
              )}
            </Group>
          ))
        ) : (
          <Text fz="sm" c="dimmed">
            No published ports.
          </Text>
        )}
        {item.public.map((p) => (
          <Group key={p.domain_id} gap="xs">
            <Anchor href={p.url} target="_blank" rel="noreferrer" fz="sm" fw="bold">
              {p.url}
            </Anchor>
            {p.status !== "ready" && (
              <StatusBadge text={p.status} intent={domainStatusIntention(p.status)} />
            )}
          </Group>
        ))}
        {!!item.public.length && <Details item={item} />}
      </Stack>
      <ServiceAccessButton item={item} />
    </Group>
  );
}

/** "Access" block on a Stack / Deployment page. */
function ServiceAccess({ target }: { target: ServiceTarget }) {
  const isStack = target.type === "Stack";
  const items = (
    useRead("ListServices", {}, { refetchInterval: 15_000 }).data ?? []
  ).filter((s) => s.target.type === target.type && s.target.id === target.id);
  const isAdmin = useUser().data?.admin ?? false;
  if (!items.length) return null;
  return (
    <Section
      title="Access"
      icon={<ICONS.Domain size="1.3rem" />}
      description={
        isAdmin
          ? "Where to open this service, and a one-click public address."
          : "Where to open this service."
      }
    >
      <Stack>
        {items.map((item) => (
          <Row key={item.service} item={item} showName={isStack} />
        ))}
      </Stack>
    </Section>
  );
}

export const StackAccess = ({ id }: { id: string }) => (
  <ServiceAccess target={{ type: "Stack", id }} />
);
export const DeploymentAccess = ({ id }: { id: string }) => (
  <ServiceAccess target={{ type: "Deployment", id }} />
);
