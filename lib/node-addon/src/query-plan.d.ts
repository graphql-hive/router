export interface QueryPlan {
  kind: "QueryPlan";
  node?: PlanNode;
  /** How the router shapes the operation's response out of the data the fetches brought. */
  projection: ResponseProjection;
}

/**
 * The operation's selections, merged by response key and with fragments resolved into type
 * conditions, the way the router walks them to write the response. Each field is projected from
 * the object of its parent type, in order:
 *
 * - A field with `parentTypes` is left out when the parent object is of none of them.
 * - A field with a `condition` that fails is left out, or is `null` for a failing `fieldType`
 *   or `enumValues` (the latter with a "Value is not a valid enum value" error).
 * - A `typename` field is the name of the parent type.
 * - A field with `selections` projects those from its object, or from each of its list's items.
 *   Other fields keep their value as it is.
 * - A missing field is `null`. A `null` in a `nonNull` field, or in a list whose `itemNonNull`
 *   says the items are non-null, makes the parent `null` instead, up to the nearest nullable one.
 *
 * A parent object's type is its `__typename`, or the schema type of the field holding it, or
 * `rootType` at the root.
 */
export interface ResponseProjection {
  /** The operation's root type, the parent type of the root `fields`. */
  rootType: string;
  fields: ProjectionField[];
}

export interface ProjectionField {
  /** The field's name in the schema. */
  name: string;
  /** Where the field goes in the response, when not under `name`. */
  alias?: string;
  /** The field is `__typename`. */
  typename?: true;
  nonNull?: true;
  /**
   * For a list field, whether the items of each list level are non-null, from the outside in:
   * `[false, true]` for `[[Int!]]`.
   */
  itemNonNull?: boolean[];
  /** The field applies only to a parent object of one of these types. */
  parentTypes?: string[];
  condition?: ProjectionCondition;
  selections?: ProjectionField[];
}

/**
 * When a field is projected. `include` and `skip` are `@include(if:)` and `@skip(if:)` with the
 * variable's name. `parentType` holds when the parent object is of one of the types, `fieldType`
 * when the field's value is. `enumValues` holds when the field's value isn't a string or is one
 * of the values. `and` holds when both do, failing like the first that fails. `or` holds when
 * either does, failing like the right one when neither does.
 */
export type ProjectionCondition =
  | { include: string }
  | { skip: string }
  | { parentType: string[] }
  | { fieldType: string[] }
  | { enumValues: string[] }
  | { and: [ProjectionCondition, ProjectionCondition] }
  | { or: [ProjectionCondition, ProjectionCondition] };

export type PlanNode =
  | FetchNode
  | BatchFetchNode
  | SequenceNode
  | ParallelNode
  | FlattenNode
  | SubscriptionNode
  | DeferNode;

/**
 * A fetch's number in its plan. Fetches name the fetches they wait for by it, in `dependsOn`.
 * It's not the fetch's position in the plan.
 */
export type FetchId = number;

export interface FetchNode {
  kind: "Fetch";
  serviceName: string;
  variableUsages?: string[];
  operationKind?: "query" | "mutation" | "subscription";
  operation: string;
  customScalarPaths?: CustomScalarPaths;
  requires?: RequiresSelection[];
  inputRewrites?: InputRewrite[];
  outputRewrites?: OutputRewrite[];
  contextRewrites?: KeyRenamer[];
  contextVariables?: string[];
  /** The fetch runs when any of these holds, always when missing. */
  runIf?: ConditionGroup[];
  id: FetchId;
  /**
   * The fetches whose data this fetch reads, it can start once they're all done. Empty for a
   * fetch that reads nothing another fetch brings, like a root fetch. An id merged into a
   * batch fetch stands for that batch, see `BatchFetchNode.completes`.
   */
  dependsOn: FetchId[];
  /**
   * The most groups of context values one request of this fetch may carry, at least 1, from the
   * `maxAliasesPerRequest` options. Put up to this many groups in one request, each under its own
   * aliases, and send as many requests as it takes. 1 means one request per group, without
   * aliasing. The planner already checked that the operation can be aliased per group, so the
   * executor doesn't have to. Only on fetches with `contextVariables`, missing on those when there
   * is no limit, so every group goes in one request.
   */
  maxContextGroups?: number;
}

/**
 * The response paths, under the fetch's response data, that hold custom scalar values.
 * A `terminal` path is a custom scalar itself.
 */
export interface CustomScalarPaths {
  children?: Record<string, CustomScalarPaths>;
  terminal?: boolean;
}

/** Holds when every `includeIf` variable is `true` and no `skipIf` one is. */
export interface ConditionGroup {
  includeIf?: string[];
  skipIf?: string[];
}

export interface BatchFetchNode {
  kind: "BatchFetch";
  serviceName: string;
  variableUsages?: string[];
  operationKind?: "query" | "mutation" | "subscription";
  operation: string;
  customScalarPaths?: CustomScalarPaths;
  entityBatch: EntityBatch;
  /** The first of the ids in `completes`. */
  id: FetchId;
  /** What the fetches merged into this batch waited for, without those fetches themselves. */
  dependsOn: FetchId[];
  /**
   * Every fetch merged into this batch. A fetch that depends on any of them depends on the batch.
   */
  completes: FetchId[];
  /**
   * Like `FetchNode.maxContextGroups`, only on batches with an alias that has
   * `contextVariables`. A group repeats every alias of the batch, so the limit is
   * `maxAliasesPerRequest` divided by the number of aliases.
   */
  maxContextGroups?: number;
}

export interface EntityBatch {
  aliases: EntityBatchAlias[];
}

export interface EntityBatchAlias {
  alias: string;
  representationsVariableName: string;
  paths: FlattenNodePathSegment[][];
  requires: RequiresSelection[];
  inputRewrites?: InputRewrite[];
  outputRewrites?: OutputRewrite[];
  contextRewrites?: KeyRenamer[];
  contextVariables?: string[];
  /** Like `FetchNode.runIf`, an alias that's off sends no representations. */
  runIf?: ConditionGroup[];
}

export interface SubscriptionNode {
  kind: "Subscription";
  primary: FetchNode;
}

export type FetchNodePathSegment =
  | "Parent"
  | { TypenameEquals: string[] }
  | { Key: string };

export type FetchRewrite =
  | { ValueSetter: ValueSetter }
  | { KeyRenamer: KeyRenamer };

export type InputRewrite = FetchRewrite;
export type OutputRewrite = FetchRewrite;

export interface ValueSetter {
  path: FetchNodePathSegment[];
  setValueTo: string;
}

export interface KeyRenamer {
  path: FetchNodePathSegment[];
  renameKeyTo: string;
}

export interface InlineFragmentRequiresNode {
  kind: "InlineFragment";
  typeCondition: string;
  selections: RequiresSelection[];
  skipIf?: string;
  includeIf?: string;
}

export interface FieldRequiresNode {
  kind: "Field";
  name: string;
  alias?: string;
  selections?: RequiresSelection[];
  omitFromResponse?: boolean;
}

export type RequiresSelection = InlineFragmentRequiresNode | FieldRequiresNode;

export interface SequenceNode {
  kind: "Sequence";
  nodes: PlanNode[];
}

export interface ParallelNode {
  kind: "Parallel";
  nodes: PlanNode[];
}

export type FlattenNodePathSegment =
  | { Field: string }
  | { TypeCondition: string[] }
  | "@";

export interface FlattenNode {
  kind: "Flatten";
  path: FlattenNodePathSegment[];
  node: PlanNode;
}

export interface DeferNode {
  kind: "Defer";
  primary: DeferPrimary;
  deferred: DeferredNode[];
}

export interface DeferPrimary {
  subselection: string | null;
  node: PlanNode | null;
}

export interface DeferredNode {
  depends: DeferDependency[];
  label?: string;
  queryPath: string[];
  subselection?: string;
  node?: PlanNode;
}

export interface DeferDependency {
  id: string;
  deferLabel?: string;
}
