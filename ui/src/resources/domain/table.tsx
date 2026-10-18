import { useResourceSelectionState } from "@/lib/hooks";
import { DataTable, SortableHeader, StatusBadge } from "mogh_ui";
import { BoxProps, Text } from "@mantine/core";
import { Types } from "komodo_client";
import ResourceLink from "@/resources/link";
import TableTags from "@/components/tags/table";
import { domainStatusIntention } from "@/lib/color";

const SORT_KEYS = ["Name"];

export default function DomainTable({
  resources,
  onServerSort,
  ...boxProps
}: {
  resources: Types.DomainListItem[];
  /** When provided, sorting is handled server side,
   * and sort updates are passed to this callback. */
  onServerSort?: (sort: { sort_by?: string; sort_desc?: boolean }) => void;
} & BoxProps) {
  const selectionState = useResourceSelectionState("Domain");
  return (
    <DataTable
      {...boxProps}
      manualSorting={!!onServerSort}
      onSortingStateChange={
        onServerSort &&
        ((sorting) => {
          const sort = sorting.find((s) => SORT_KEYS.includes(s.id));
          onServerSort(sort ? { sort_by: sort.id, sort_desc: sort.desc } : {});
        })
      }
      tableKey="domains"
      data={resources}
      selectOptions={{
        selectKey: ({ name }) => name,
        state: selectionState,
      }}
      columns={[
        {
          id: "Name",
          accessorKey: "name",
          header: ({ column }) => (
            <SortableHeader column={column} title="Name" />
          ),
          cell: ({ row }) => (
            <ResourceLink type="Domain" id={row.original.id} />
          ),
        },
        {
          header: "Hostname",
          accessorKey: "info.hostname",
        },
        {
          header: "DNS",
          cell: ({ row }) =>
            row.original.info.dns_account || <Text c="dimmed">-</Text>,
        },
        {
          header: "Ingress",
          cell: ({ row }) =>
            row.original.info.ingress_account || <Text c="dimmed">-</Text>,
        },
        {
          header: "Status",
          cell: ({ row }) => (
            <StatusBadge
              text={row.original.info.status_code}
              intent={domainStatusIntention(row.original.info.status_code)}
            />
          ),
        },
        {
          header: "Tags",
          cell: ({ row }) => <TableTags tagIds={row.original.tags} />,
        },
      ]}
    />
  );
}
