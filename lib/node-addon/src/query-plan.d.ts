export interface QueryPlan {
  kind: "QueryPlan";
  node?: PlanNode;
  representationReusePlan?: RepresentationReusePlan;
}

export interface RepresentationReusePlan {
  version: number;
  groups: number[][];
}

export type PlanNode =
  | FetchNode
  | SequenceNode
  | ParallelNode
  | FlattenNode
  | ConditionNode
  | SubscriptionNode
  | DeferNode;

export interface FetchNode {
  kind: "Fetch";
  id: number;
  serviceName: string;
  variableUsages?: string[];
  operationKind?: "query" | "mutation" | "subscription";
  operationName?: string;
  operation: string;
  requires?: RequiresSelection[];
  inputRewrites?: InputRewrite[];
  outputRewrites?: OutputRewrite[];
}

export interface SubscriptionNode {
  kind: "Subscription";
  primary: PlanNode;
}

export type FetchNodePathSegment = { TypenameEquals: string } | { Key: string };

export type FetchRewrite =
  | { ValueSetter: ValueSetter }
  | { KeyRenamer: KeyRenamer }
  // TODO: why sometimes? see query plans of federation tests in hive gateway
  | ({ kind: "ValueSetter" } & ValueSetter);

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
}

export interface FragmentSpreadRequiresNode {
  kind: "FragmentSpread";
  value: string;
}

export type RequiresSelection =
  | InlineFragmentRequiresNode
  | FieldRequiresNode
  | FragmentSpreadRequiresNode;

export interface SequenceNode {
  kind: "Sequence";
  nodes: PlanNode[];
}

export interface ParallelNode {
  kind: "Parallel";
  nodes: PlanNode[];
}

export type FlattenNodePathSegment = { Field: string } | { Cast: string } | "@";

export interface FlattenNode {
  kind: "Flatten";
  path: FlattenNodePathSegment[];
  node: PlanNode;
}

export interface ConditionNode {
  kind: "Condition";
  condition: string;
  ifClause?: PlanNode;
  elseClause?: PlanNode;
}

export interface DeferNode {
  kind: "Defer";
  primary: DeferPrimary;
  deferred: DeferredNode[];
}

export interface DeferPrimary {
  subselection?: string;
  node?: PlanNode;
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
