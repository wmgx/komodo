import { Types } from "komodo_client";

/** Older persisted bindings may only have the normalized ASCII hostname. */
export function displayHostname(domain: Types.Domain): string {
  const display =
    domain.info && "display_hostname" in domain.info
      ? domain.info.display_hostname
      : undefined;
  return typeof display === "string" && display
    ? display
    : (domain.config?.hostname ?? "");
}
