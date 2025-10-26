export interface QueryPlan {
  kind: "QueryPlan";
  node?: PlanNode;
}

export type PlanNode =
  | FetchNode
  | SequenceNode
  | ParallelNode
  | FlattenNode
  | ConditionNode
  | SubscriptionNode;

export interface FetchNode {
  kind: "Fetch";
  serviceName: string;
  variableUsages: string[];
  operationKind?: "query" | "mutation" | "subscription";
  operationName?: string;
  operation: string;
  requires?: InlineFragmentRequiresNode[];
  inputRewrites?: InputRewrite[];
  outputRewrites?: OutputRewrite[];
}

export interface SubscriptionNode {
  kind: "Subscription";
  primary: PlanNode;
}

export type FetchNodePathSegment = { TypenameEquals: string } | { Key: string };

export type InputRewrite =
  | { [kind in "ValueSetter"]: ValueSetter }
  | (ValueSetter & { kind: "ValueSetter" });

export interface ValueSetter {
  path: FetchNodePathSegment[];
  setValueTo: string;
}

export type OutputRewrite =
  // TODO: why InputRewrite has `(ValueSetter & { kind: "ValueSetter" })` but OutputRewrite doesn't have `(KeyRenamer & { kind: "KeyRenamer" })`?
  { [kind in "KeyRenamer"]: KeyRenamer };

export interface KeyRenamer {
  path: FetchNodePathSegment[];
  renameKeyTo: string;
}

export interface InlineFragmentRequiresNode {
  kind: "InlineFragment";
  typeCondition: string;
  selections: RequiresSelection[];
}

export interface FieldRequiresNode {
  kind: "Field";
  name: string;
  selections?: RequiresSelection[];
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

export type FlattenNodePathSegment = { Field: string } | { Cast: string } | "@";

export interface FlattenNode {
  kind: "Flatten";
  path: FlattenNodePathSegment[];
  node: PlanNode;
}

export interface ConditionNode {
  kind: "Condition";
  condition: string;
  ifClause: PlanNode;
  elseClause?: PlanNode;
}
