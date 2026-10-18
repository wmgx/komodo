import { useInvalidate, useRead, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import {
  Button,
  Group,
  Modal,
  PasswordInput,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { CircleCheckBig } from "lucide-react";
import { MonacoEditor } from "mogh_ui";
import { Types } from "komodo_client";
import { useState } from "react";

/** Create (no `account`) or edit (with `account`) a Domain account. */
export default function DomainAccountForm({
  account,
}: {
  account?: Types.DomainAccount;
}) {
  const editing = !!account;
  const [opened, { open, close }] = useDisclosure();
  const kinds = useRead("ListDomainProviderKinds", {}, { enabled: opened }).data;

  const [name, setName] = useState(account?.name ?? "");
  const [kind, setKind] = useState(account?.kind ?? "");
  const [token, setToken] = useState("");
  const [params, setParams] = useState(
    JSON.stringify(account?.params ?? {}, undefined, 2),
  );
  const [description, setDescription] = useState(account?.description ?? "");

  const inv = useInvalidate();
  const onSuccess = () => {
    inv(["ListDomainAccounts"], ["GetDomainAccount"]);
    notifications.show({
      message: editing ? "Account updated" : "Account created",
    });
    setToken("");
    close();
  };
  const { mutate: create, isPending: createPending } = useWrite(
    "CreateDomainAccount",
    { onSuccess },
  );
  const { mutate: update, isPending: updatePending } = useWrite(
    "UpdateDomainAccount",
    { onSuccess },
  );

  const fillExample = () => {
    const example = kinds?.find((k) => k.kind === kind)?.params_example;
    if (example) setParams(JSON.stringify(example, undefined, 2));
  };

  const submit = () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(params || "{}");
    } catch {
      notifications.show({ message: "Params is not valid JSON", color: "red" });
      return;
    }
    if (editing) {
      update({
        id: account.id,
        // Empty token means "do not change"
        token: token || undefined,
        params: parsed,
        description,
      });
    } else {
      if (!name.trim() || !kind || !token) {
        notifications.show({
          message: "Name, kind and token are required",
          color: "red",
        });
        return;
      }
      create({ name: name.trim(), kind, token, params: parsed, description });
    }
  };

  return (
    <>
      <Modal
        opened={opened}
        onClose={close}
        title={
          <Text size="lg">
            {editing ? `Edit ${account.name}` : "Create Domain Account"}
          </Text>
        }
        size="lg"
      >
        <Stack>
          <TextInput
            label="Name"
            placeholder="cloudflare-main"
            value={name}
            onChange={(e) => setName(e.target.value)}
            disabled={editing}
          />
          <Select
            label="Kind"
            value={kind || null}
            onChange={(k) => setKind(k ?? "")}
            data={kinds?.map((k) => k.kind) ?? (kind ? [kind] : [])}
            disabled={editing}
          />
          <PasswordInput
            label="Token"
            description={
              editing
                ? `Currently ${account.token_set ? "set" : "not set"}. Leave empty to keep it unchanged.`
                : "Provider API token. Never shown again after saving."
            }
            placeholder={editing ? "Unchanged" : "API token"}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            autoComplete="new-password"
          />
          <Stack gap="xs">
            <Group justify="space-between">
              <Text size="sm" fw={500}>
                Params (JSON)
              </Text>
              <Button
                size="xs"
                variant="default"
                onClick={fillExample}
                disabled={!kind}
              >
                Fill example
              </Button>
            </Group>
            <MonacoEditor
              value={params}
              language="json"
              onValueChange={setParams}
            />
          </Stack>
          <TextInput
            label="Description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
          <Group justify="end">
            <Button
              leftSection={<CircleCheckBig size="1rem" />}
              onClick={submit}
              loading={createPending || updatePending}
            >
              {editing ? "Save" : "Create"}
            </Button>
          </Group>
        </Stack>
      </Modal>

      <Button
        leftSection={
          editing ? <ICONS.Edit size="1rem" /> : <ICONS.Create size="1rem" />
        }
        variant={editing ? "default" : "filled"}
        onClick={open}
      >
        {editing ? "Edit" : "New Account"}
      </Button>
    </>
  );
}
