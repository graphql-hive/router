// Runs an operation against the deterministic data, as one subgraph (`graph` set) or as the
// monolith (`graph` null). As a subgraph it also checks what the router asks for.

import { execute, getOperationAST, validate } from 'graphql';

import { FIELD_ERROR, stableJson } from './data.js';

// what a field the data made fail says
export const INJECTED = /^[A-Za-z0-9_]+\.[A-Za-z0-9_]+ failed$/;

export function createExecutor(supergraph, data) {
  const { schema, resolves, resolvesEntity, requires } = supergraph;

  function resolveEntities(representations, context, info) {
    const responseKey = info.fieldNodes[0].alias?.value ?? info.fieldName;
    const seen = new Set();
    return representations.map((representation, index) => {
      const typeName = representation.__typename;
      const json = stableJson(representation);
      if (seen.has(json)) context.issue('duplicate-representation', `${typeName} in ${responseKey}`);
      seen.add(json);
      if (!resolvesEntity(typeName, context.graph)) context.issue('entity-not-resolvable', typeName);
      const ref = data.decodeEntity(typeName, representation);
      if (!ref) {
        context.issue('undecodable-representation', json, data.whyUndecodable(typeName, representation));
        return null;
      }
      for (const difference of data.verifyRepresentation(ref, representation)) {
        context.issue('wrong-representation', difference);
      }
      context.lookups.push({
        responseKey,
        index,
        entity: `${ref.__typename}:${ref.seed}`,
        // an `@interfaceObject` subgraph answers with the interface it was asked about. One with
        // an entity interface answers with the object type
        sentAs: supergraph.isInterfaceObjectIn(typeName, context.graph) ? typeName : null,
      });
      return ref;
    });
  }

  /** Whether a field above `ref` `@provides` its `fieldName` in `graph`. */
  function provided(ref, fieldName, graph) {
    const path = [fieldName];
    for (let object = ref; object?.via; object = object.parent) {
      let tree = supergraph.provides(object.via, graph);
      for (const step of path) tree = tree && step in tree ? (tree[step] ?? {}) : null;
      if (tree) return true;
      path.unshift(object.via.split('.')[1]);
    }
    return false;
  }

  function fieldResolver(source, args, context, info) {
    const parentName = info.parentType.name;
    if (info.fieldName === '_entities') return resolveEntities(args.representations, context, info);
    if (
      context.graph &&
      !resolves(parentName, info.fieldName, context.graph) &&
      !provided(source, info.fieldName, context.graph)
    ) {
      context.issue('not-owned', `${parentName}.${info.fieldName}`);
    }
    // a subgraph that has a `@requires` field's inputs itself, a `@provides` gave them, reads them
    // from its own data. The router never sees them, they're still used
    const required = context.graph && !source.representation && requires(parentName, info.fieldName);
    if (required) context.localInputs.push(data.project(source, required));
    const value = data.fieldValue(source, parentName, info.fieldName, info.returnType, args);
    if (value === FIELD_ERROR) throw new Error(`${parentName}.${info.fieldName} failed`);
    return value;
  }

  /**
   * `{ result, issues, lookups, localInputs }`. `lookups` has one entry per representation: the
   * entity and whether the router asked anything of it. `localInputs` are the `@requires` inputs
   * a subgraph read from its own data.
   */
  async function run({ document, variables, operationName, graph = null }) {
    const issues = [];
    const lookups = [];
    const localInputs = [];
    const context = {
      graph,
      lookups,
      localInputs,
      issue: (kind, detail, causes) => issues.push({ kind, detail, causes }),
    };

    const errors = validate(schema, document);
    if (errors.length) {
      for (const error of errors.slice(0, 5)) context.issue('invalid-operation', error.message);
      return { result: { errors: errors.map((error) => ({ message: error.message })) }, issues, lookups, localInputs };
    }

    const operation = getOperationAST(document, operationName);
    const rootType = schema.getRootType(operation.operation);
    const result = await execute({
      schema,
      document,
      operationName,
      variableValues: variables ?? {},
      rootValue: { __typename: rootType.name, seed: 0 },
      contextValue: context,
      fieldResolver,
      typeResolver: (value) => value.__typename,
    });

    for (const lookup of lookups) {
      const entity = result.data?.[lookup.responseKey]?.[lookup.index];
      if (lookup.sentAs && entity?.__typename) entity.__typename = lookup.sentAs;
      // the router sent the entity, but there was nothing to ask of it. A null one failed, that's
      // a fault the data made
      if (entity && !Object.keys(entity).some((key) => key !== '__typename')) {
        context.issue('empty-lookup', lookup.entity);
      }
    }
    for (const error of result.errors ?? []) {
      if (!INJECTED.test(error.message)) context.issue('execution-error', error.message);
    }

    return { result: JSON.parse(JSON.stringify(result)), issues, lookups, localInputs };
  }

  return { run };
}
