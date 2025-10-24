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

export type OutputRewrite = KeyRenamer;

export type FetchNodePathSegment = { TypenameEquals: string } | { Key: string };

export interface KeyRenamer {
  kind: "KeyRenamer";
  path: FetchNodePathSegment[];
  renameKeyTo: string;
}

export type InputRewrite = ValueSetter;

export interface ValueSetter {
  kind: "ValueSetter";
  path: FetchNodePathSegment[];
  setValueTo: any;
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
