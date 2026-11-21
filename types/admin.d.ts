// The JSON shapes of the service that the administration pages read: `static/js/pages/admin.js`,
// `static/js/pages/account.js`, `static/js/webhooks.js` and `static/js/audit.js`. A field is
// optional or nullable here where the scripts guard it; the authority for each shape is the
// Rust type or handler named beside it. This file has no import or export, which keeps its
// declarations global.

/** A row of `GET /admin/users` (`admin_list_users` in `src/presentation/handlers/admin.rs`). */
interface AdminUserRow {
  id: number;
  username: string;
  approved: boolean;
  service_account?: boolean;
  /** `AccountKind` in `src/domain/models.rs`: service, local, ldap, oidc or unknown. */
  kind: string;
  display_name?: string | null;
  gravatar_hash?: string | null;
  roles?: string[];
}

/** `RoleCatalogueEntry` in `src/presentation/handlers/roles.rs`. */
interface RoleCatalogueEntry {
  role: string;
  permissions: string[];
  globally_grantable: boolean;
}

/** `GET /admin/roles`. */
interface RoleCatalogue {
  roles?: RoleCatalogueEntry[];
}

/** `GroupDetail` in `src/application/authz.rs`: a group as `GET /admin/groups` lists it. */
interface GroupDetail {
  id: number;
  name: string;
  /** `GroupSource` in `src/domain/models.rs`: native or ldap. */
  source: string;
  roles?: string[];
  member_ids?: number[];
}

/** `GET /admin/groups`. */
interface GroupList {
  groups?: GroupDetail[];
}

/** `DirectoryGroupHit` in `src/application/authz.rs`. */
interface DirectoryGroupHit {
  dn: string;
  name: string;
  /** The Sanshain group of this directory group, or null while it has none. */
  group_id: number | null;
}

/** `GET /admin/directory-groups`. */
interface DirectoryGroupHits {
  groups?: DirectoryGroupHit[];
}

/** `ProducerMaintainers` in `src/application/authz.rs`. */
interface ProducerMaintainers {
  producer: string;
  user_ids?: number[];
  group_ids?: number[];
}

/** `GET /admin/maintainers`. */
interface MaintainerList {
  producers?: ProducerMaintainers[];
}

/** The cache part of `GET /admin/instance-settings`. */
interface CacheSettings {
  enabled: boolean;
  memory_limit_mb: number;
  memory_used_mb: number;
  entry_count: number;
  hit_count: number;
  miss_count: number;
  hit_rate: number;
}

/** `GET /admin/instance-settings` (`get_instance_settings` in `src/presentation/handlers/admin.rs`). */
interface InstanceSettings {
  auto_approve_users?: boolean;
  registration_enabled?: boolean;
  gravatar_enabled?: boolean;
  dev_mode_allowed?: boolean;
  snapshot_max_age_days: number;
  dependency_max_age_days: number;
  trunk_max_age_days: number;
  ticket_url_template?: string | null;
  cache: CacheSettings;
}

/** `GET /admin/settings/database`. */
interface DatabaseInfo {
  backend: string;
  url: string;
}

/** `AllowedHostsPayload` in `src/presentation/handlers/webhooks.rs`. */
interface WebhookAllowedHosts {
  hosts?: string[];
}

/** `BranchInfo` in `src/domain/models.rs`: a sanshain-branch as `GET /admin/branches` lists it. */
interface BranchInfo {
  id: number;
  name: string;
  created_at: string;
  created_by: string;
  source: string;
  as_of?: string | null;
}

/** `LdapConfig` in `src/domain/models.rs`, as `GET /admin/auth-config` answers it. */
interface LdapConfigView {
  server_url?: string;
  bind_dn?: string;
  bind_password?: string | null;
  base_dn?: string;
  user_filter?: string;
  username_attribute?: string;
  admin_group?: string;
}

/** `GET /admin/auth-config`. The identity provider's fields are read by their names. */
interface AuthConfigView {
  /** `AuthMode` in `src/domain/models.rs`: disabled, dev, local, ldap or oidc. */
  auth_mode: string;
  ldap_config?: LdapConfigView | null;
  oidc_config?: Record<string, string | null | undefined> | null;
}

/** `ProvideResponse` in `src/domain/models.rs`, as far as the admin page shows it. */
interface ProvideResult {
  version: string;
  stability: string;
  changes?: { inserts?: number; updates?: number; deletes?: number };
}

/** `Webhook` in `src/domain/models.rs`. */
interface Webhook {
  id: number;
  url: string;
  /** `WebhookFormat`: json, slack or teams. */
  format: string;
  event_types?: string[] | null;
  producer_glob?: string | null;
  secret_hint: string;
  paused: boolean;
  allow_list_exempt: boolean;
  created_by: string;
  created_at?: string | null;
}

/** `CreatedWebhook` in `src/application/webhook_service.rs`: a hook with its secret, shown once. */
interface WebhookSecret {
  secret: string;
}

/** `WebhookDelivery` in `src/domain/models.rs`. */
interface WebhookDelivery {
  id: number;
  event_type: string;
  occurred_at?: string | null;
  /** `WebhookDeliveryStatus`: pending, delivered, failed or dead. */
  status: string;
  /** The script converts it with `Number()` and shows 0 for anything else. */
  attempts?: number | null;
  next_attempt_at?: string | null;
  last_response_code?: number | null;
  last_error?: string | null;
}

/** `AuditLogView` in `src/application/auth_service.rs`: a row of `GET /api/audit/timeline`. */
interface AuditLogView {
  id: number;
  timestamp: string;
  username: string;
  display_name?: string | null;
  action: string;
  details: string;
  service?: string | null;
  version?: string | null;
  action_type?: string | null;
  diff?: string | null;
}

/** `LoginOptions` in `src/application/sign_in_service.rs`: `GET /auth/options`. */
interface LoginOptions {
  auth_mode?: string;
  registration: boolean;
  auto_approve: boolean;
  sso: boolean;
  admins_only?: boolean;
}

/** `ListedToken` in `src/application/service_account_service.rs`: a row of `GET /auth/tokens`. */
interface ListedToken {
  id: string;
  name: string;
  /** `ApiTokenScope` in `src/domain/models.rs`: full or read_only. */
  scope: string;
  created_at: string;
  expires_at: string;
  last_used_at?: string | null;
  issued_by?: string | null;
  for_user?: string | null;
}
