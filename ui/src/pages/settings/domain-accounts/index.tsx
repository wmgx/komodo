import { useInvalidate, useRead, useSetTitle, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import { ConfirmButton, DataTable, Section, SortableHeader } from "mogh_ui";
import { Anchor, Badge, Group, Stack, Text } from "@mantine/core";
import { Link } from "react-router-dom";
import { notifications } from "@mantine/notifications";
import DomainAccountForm from "./form";

export default function SettingsDomainAccounts() {
  useSetTitle("Domain Accounts");
  const accounts = useRead("ListDomainAccounts", {}).data ?? [];

  const inv = useInvalidate();
  const { mutate: del, isPending } = useWrite("DeleteDomainAccount", {
    onSuccess: () => {
      inv(["ListDomainAccounts"], ["GetDomainAccount"]);
      notifications.show({ message: "Account deleted" });
    },
  });

  return (
    <Stack gap="xl">
      <Section
        title="Domain Accounts"
        icon={<ICONS.Domain size="1.3rem" />}
        description="Add a Cloudflare account once. Then open Services and click Make public. Tokens are write-only."
      >
        <Group>
          <DomainAccountForm />
        </Group>

        <DataTable
          tableKey="domain-accounts"
          data={accounts}
          columns={[
            {
              accessorKey: "name",
              header: ({ column }) => (
                <SortableHeader column={column} title="Name" />
              ),
            },
            { accessorKey: "kind", header: "Kind" },
            {
              header: "Capabilities",
              cell: ({ row }) => (
                <Group gap="xs">
                  {row.original.capabilities.map((c) => (
                    <Badge key={c} variant="default">
                      {c}
                    </Badge>
                  ))}
                </Group>
              ),
            },
            {
              header: "Token",
              cell: ({ row }) => (
                <Badge color={row.original.token_set ? "green" : "red"}>
                  {row.original.token_set ? "Set" : "Not set"}
                </Badge>
              ),
            },
            {
              header: "Description",
              cell: ({ row }) => (
                <Text fz="sm" c="dimmed">
                  {row.original.description}
                </Text>
              ),
            },
            {
              header: "Edit",
              cell: ({ row }) =>
                row.original.source === "Database" ? (
                  <DomainAccountForm account={row.original} />
                ) : (
                  <Text c="dimmed" fz="sm">
                    From config
                  </Text>
                ),
            },
            {
              header: "Delete",
              cell: ({ row }) =>
                row.original.source === "Database" && (
                  <ConfirmButton
                    icon={<ICONS.Delete size="1rem" />}
                    onClick={() => del({ id: row.original.id })}
                    loading={isPending}
                    confirmProps={{ variant: "filled", color: "red" }}
                  >
                    Delete
                  </ConfirmButton>
                ),
            },
          ]}
        />
        <Text fz="sm" c="dimmed">
          Advanced:{" "}
          <Anchor component={Link} to="/domains" fz="sm">
            Domains
          </Anchor>{" "}
          (every address Komodo manages, with plans and details)
        </Text>
      </Section>
    </Stack>
  );
}
