import { useExecute, usePermissions, useRead } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import { ConfirmButton, DataTable, Section, StatusBadge } from "mogh_ui";
import { Alert, Button, Checkbox, Code, Group, Stack, Text } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Types } from "komodo_client";
import { useState } from "react";

const ACTION_INTENT = {
  Create: "Good",
  Update: "Warning",
  Delete: "Critical",
  Noop: "Neutral",
  Conflict: "Critical",
} as const;

function fmtValue(value: unknown) {
  return value === null || value === undefined ? "-" : JSON.stringify(value);
}

function Reasons({ reasons }: { reasons: Types.DomainReason[] }) {
  return (
    <Stack gap="xs">
      {reasons.map((r, i) => (
        <Alert key={i} color="red" title={r.code} p="xs">
          {r.message}
          {r.object_key ? ` (${r.object_key})` : ""}
        </Alert>
      ))}
    </Stack>
  );
}

export default function DomainPlan({ id }: { id: string }) {
  const { canExecute } = usePermissions({ type: "Domain", id });
  const [adopt, setAdopt] = useState(false);

  // Planning calls the provider, so only run it on demand.
  const planQuery = useRead(
    "GetDomainPlan",
    { domain: id },
    { enabled: false, retry: false, staleTime: Infinity },
  );
  const plan = planQuery.data;
  const { mutate: apply, isPending } = useExecute("ApplyDomain", {
    onSuccess: (update) => {
      notifications.show({
        message: update.success ? "Applied Domain" : "Apply failed, see Updates",
        color: update.success ? "green" : "red",
      });
      setAdopt(false);
      planQuery.refetch();
    },
  });

  const changes = plan?.changes ?? [];
  const hasConflict = changes.some(
    (c) => c.action === Types.ChangeAction.Conflict,
  );
  const hasWork = changes.some((c) => c.action !== Types.ChangeAction.Noop);
  const canApply =
    canExecute &&
    !!plan &&
    plan.errors.length === 0 &&
    hasWork &&
    (!hasConflict || adopt);

  return (
    <Section
      title="Plan"
      icon={<ICONS.Domain size="1.3rem" />}
      description="Preview what Apply will change on the provider."
      actions={
        <Button
          leftSection={<ICONS.Refresh size="1rem" />}
          loading={planQuery.isFetching}
          onClick={() => {
            setAdopt(false);
            planQuery.refetch();
          }}
        >
          {plan ? "Refresh plan" : "Preview plan"}
        </Button>
      }
    >
      {!!planQuery.error && (
        <Text c="red">
          {(planQuery.error as { result?: { error?: string } }).result?.error ??
            "Failed to compute plan"}
        </Text>
      )}
      {plan && (
        <Stack>
          {plan.errors.length > 0 && <Reasons reasons={plan.errors} />}

          {plan.errors.length === 0 && !hasWork && (
            <Text c="dimmed">No changes. Remote matches the config.</Text>
          )}

          {changes.length > 0 && (
            <DataTable
              tableKey="domain-plan"
              data={changes}
              columns={[
                {
                  header: "Object",
                  cell: ({ row }) => (
                    <Text fz="sm">
                      {row.original.object.kind}{" "}
                      <Code>{row.original.object.key}</Code>
                    </Text>
                  ),
                },
                {
                  header: "Action",
                  cell: ({ row }) => (
                    <StatusBadge
                      text={row.original.action}
                      intent={ACTION_INTENT[row.original.action]}
                    />
                  ),
                },
                {
                  header: "Before",
                  cell: ({ row }) => <Code>{fmtValue(row.original.before)}</Code>,
                },
                {
                  header: "After",
                  cell: ({ row }) => <Code>{fmtValue(row.original.after)}</Code>,
                },
                {
                  header: "Reason",
                  cell: ({ row }) => row.original.reason ?? "-",
                },
              ]}
            />
          )}

          {hasConflict && (
            <Checkbox
              checked={adopt}
              onChange={(e) => setAdopt(e.currentTarget.checked)}
              color="red"
              label="Conflicts: these remote objects already exist and are not managed by Komodo. I understand Apply will take ownership and overwrite them."
            />
          )}

          {canExecute && (
            <Group>
              <ConfirmButton
                icon={<ICONS.Run size="1rem" />}
                loading={isPending}
                disabled={!canApply || isPending}
                onClick={() =>
                  apply({
                    domain: id,
                    plan_id: plan.plan_id,
                    adopt_conflicts: hasConflict && adopt,
                  })
                }
              >
                Apply
              </ConfirmButton>
            </Group>
          )}
        </Stack>
      )}
    </Section>
  );
}
