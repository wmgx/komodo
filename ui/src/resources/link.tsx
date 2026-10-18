import { usableResourcePath } from "@/lib/utils";
import { ResourceComponents, UsableResource } from ".";
import { Group, GroupProps, Text, TextProps } from "@mantine/core";
import { Link } from "react-router-dom";
import { Types } from "komodo_client";
import { useRead } from "@/lib/hooks";
import { Globe } from "lucide-react";
import { TemplateMarker } from "@/components/template-marker";

export interface ResourceLinkProps extends TextProps {
  type: Exclude<Types.ResourceTarget["type"], "System">;
  id: string;
  onClick?: () => void;
  noColor?: boolean;
  groupProps?: GroupProps;
  iconSize?: string | number;
  /** The id field expects the resource name */
  useName?: boolean;
}

export default function ResourceLink(props: ResourceLinkProps) {
  if (
    props.type === "Domain" ||
    props.type === "DnsProvider" ||
    props.type === "Ingress"
  )
    return <DomainResourceLink {...props} type={props.type} />;
  return <LegacyResourceLink {...props} type={props.type} />;
}

function DomainResourceLink({
  type,
  id,
  onClick,
  groupProps,
  iconSize,
  noColor: _noColor,
  useName: _useName,
  ...textProps
}: ResourceLinkProps & { type: "Domain" | "DnsProvider" | "Ingress" }) {
  const domain = useRead(
    "GetDomain",
    { domain: id },
    { enabled: type === "Domain" },
  );
  const provider = useRead(
    "GetDnsProvider",
    { dns_provider: id },
    { enabled: type === "DnsProvider" },
  );
  const ingress = useRead(
    "GetIngress",
    { ingress: id },
    { enabled: type === "Ingress" },
  );
  const resource =
    type === "Domain"
      ? domain.data
      : type === "DnsProvider"
        ? provider.data
        : ingress.data;
  const path =
    type === "Domain"
      ? `/domains/${id}`
      : type === "DnsProvider"
        ? `/dns-providers?selected=${id}`
        : `/ingresses?selected=${id}`;
  return (
    <Group {...groupProps} gap="xs">
      <Globe size={iconSize ?? "1rem"} />
      <Text {...textProps} component={Link} to={path} onClick={onClick}>
        {resource?.name ?? id}
      </Text>
    </Group>
  );
}

function LegacyResourceLink({
  type,
  id,
  onClick,
  noColor,
  groupProps,
  iconSize,
  useName,
  ...textProps
}: ResourceLinkProps & { type: UsableResource }) {
  const RC = ResourceComponents[type];
  const resource = RC.useListItem(id, useName);
  return (
    <Group
      title={`${type} - ${resource?.name}`}
      renderRoot={(props) => (
        <Link to={`/${usableResourcePath(type)}/${id}`} {...props} />
      )}
      onClick={(e) => {
        e.stopPropagation();
        onClick?.();
      }}
      wrap="nowrap"
      gap="xs"
      {...groupProps}
    >
      <RC.Icon
        id={useName ? resource?.id : id}
        noColor={noColor}
        size={iconSize}
      />
      <Text
        className="hover-underline"
        style={{ textWrap: "nowrap" }}
        fz={{ base: "sm", lg: "md" }}
        {...textProps}
      >
        {resource?.name ?? "Unknown"}
      </Text>
      {resource?.template && <TemplateMarker type={type} />}
    </Group>
  );
}
