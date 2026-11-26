// The JSON shapes of the service that more than one page script reads. A field is optional or
// nullable here where the scripts guard it; the authority for each shape is the Rust type or
// handler named beside it. These are global declarations: this file has no import or export.

/** The signed-in user, as `GET /auth/me` answers (`src/presentation/handlers/auth.rs`). */
interface SanshainUser {
  id?: number;
  username: string;
  display_name?: string | null;
  gravatar_hash?: string | null;
  approved?: boolean;
  is_root?: boolean;
  roles?: string[];
  permissions?: string[];
  /** The Producers this user maintains. */
  maintains?: string[];
  /** The Products whose members this user may edit. */
  maintains_products?: string[];
  sign_in?: string;
  display_name_editable?: boolean;
  own_gravatar?: boolean;
  gravatar_enabled?: boolean;
}

/** What an avatar and a person's label are drawn from: a user, or a row that names one. */
interface PersonLike {
  username?: string | null;
  display_name?: string | null;
  gravatar_hash?: string | null;
}

/** `SystemOwner` in `src/domain/models.rs`. */
interface SystemOwner {
  kind: string;
  name: string;
}

/** `SystemLink` in `src/domain/models.rs`. */
interface SystemLink {
  url: string;
  title: string;
  type?: string | null;
  icon?: string | null;
}

/** `SystemContext` in `src/domain/models.rs`: what a system's catalog-info said about it. */
interface SystemContext {
  owner?: SystemOwner | null;
  description?: string | null;
  source_location?: string | null;
  links?: SystemLink[] | null;
  tags?: string[] | null;
}

/** `VersionOrigin` in `src/domain/models.rs`: what the providing build said about a version. */
interface VersionOrigin {
  build_url?: string | null;
  change_url?: string | null;
  tickets?: string[] | null;
  note?: string | null;
  trunk_build?: boolean;
  git_branch?: string | null;
  git_target_branch?: string | null;
  /** What the branch naming patterns read, computed when shown (san-1029). */
  branch_reading?: { kind?: string | null; target?: string | null } | null;
}

/** `Annotation` in `src/domain/models.rs`. */
interface Annotation {
  name: string;
  value: string;
}

/** `GET /version`. */
interface VersionInfo {
  version: string;
  instance_id: string;
}

/** `ProducerVersionInfo` in `src/domain/models.rs`: one entry of a Producer's version line. */
interface ProducerVersionInfo {
  api_type: string;
  version: string;
  stability: string;
  content_hash?: string;
  provided_by?: string;
  created_at?: string;
  updated_at?: string;
  last_required_at?: string | null;
  endpoint_count?: number;
  origin?: VersionOrigin | null;
  annotations?: Annotation[] | null;
  expires_at?: string | null;
  trunk_provided_at?: string | null;
}

/** `ProducerSummary` in `src/domain/models.rs`: a Producer as `GET /admin/producers` lists it. */
interface ProducerSummary {
  name: string;
  versions?: ProducerVersionInfo[];
  is_favorite?: boolean;
  icon?: string | null;
  domain?: string | null;
  external?: boolean;
}

/** `UserFavoritesResponse` in `src/domain/models.rs`. */
interface UserFavorites {
  services?: string[];
  clients?: string[];
}

/** `DiffLine` in `src/application/text_diff.rs`. */
interface DiffLine {
  op: "add" | "del" | "ctx";
  old_no?: number | null;
  new_no?: number | null;
  text: string;
  old_text?: string;
}

/** `DiffHunk` in `src/application/text_diff.rs`. */
interface DiffHunk {
  old_start: number;
  old_len: number;
  new_start: number;
  new_len: number;
  lines: DiffLine[];
}

/** The JSON diff of `GET /admin/producers/{name}/diff?format=json`. */
interface TextDiff {
  hunks: DiffHunk[];
}
