// What the harness needs to know about a supergraph: the schema to execute against, and from the
// `join__` directives which subgraph owns which field, the keys of each entity, and which members of
// an abstract type each subgraph knows.

import {
  buildASTSchema,
  getNamedType,
  isAbstractType,
  isInterfaceType,
  isUnionType,
  Kind,
  parse,
  parseType,
  print,
  valueFromASTUntyped,
  visit,
} from 'graphql';

export function isInaccessible(node) {
  return (node?.directives ?? []).some((directive) => directive.name.value === 'inaccessible');
}

function directiveArgs(node, name) {
  return (node.directives ?? [])
    .filter((directive) => directive.name.value === name)
    .map((directive) =>
      Object.fromEntries(
        (directive.arguments ?? []).map((arg) => [arg.name.value, valueFromASTUntyped(arg.value)]),
      ),
    );
}

/**
 * A field set as a tree, `{ field: subtree | null }`, `null` for a leaf. Type conditions are left
 * out, their fields count for the type they're on.
 */
function fieldSetTree(fieldSet) {
  const toTree = (selectionSet) => {
    let tree = {};
    for (const selection of selectionSet.selections) {
      if (selection.kind === Kind.INLINE_FRAGMENT) {
        tree = mergeTrees(tree, toTree(selection.selectionSet));
      } else {
        tree[selection.name.value] = selection.selectionSet ? toTree(selection.selectionSet) : null;
      }
    }
    return tree;
  };
  return toTree(parse(`{ ${fieldSet} }`).definitions[0].selectionSet);
}

function mergeTrees(left, right) {
  const merged = { ...left };
  for (const [name, subtree] of Object.entries(right)) {
    merged[name] = subtree && merged[name] ? mergeTrees(merged[name], subtree) : (subtree ?? merged[name] ?? null);
  }
  return merged;
}

export function loadSupergraph(sdl) {
  const document = parse(sdl);

  // the router asks subgraphs for `_entities`, which the supergraph doesn't have
  const entityTypes = [];
  const graphNames = {};
  const joinTypes = new Map();
  const joinFields = new Map();
  const implementsIn = new Map();
  const unionMembers = new Map();

  for (const definition of document.definitions) {
    if (definition.kind === Kind.ENUM_TYPE_DEFINITION && definition.name.value === 'join__Graph') {
      for (const value of definition.values ?? []) {
        graphNames[value.name.value] = directiveArgs(value, 'join__graph')[0].name;
      }
    }
  }
  const graph = (enumValue) => graphNames[enumValue] ?? enumValue;
  const inaccessibleTypes = new Set();
  // the types setting each `@context`
  const contextTypes = new Map();
  // `@fromContext` arguments, by field coordinate
  const contextArguments = new Map();

  for (const definition of document.definitions) {
    const typeName = definition.name?.value;
    if (!typeName) continue;
    const types = directiveArgs(definition, 'join__type').map((args) => ({ ...args, graph: graph(args.graph) }));
    joinTypes.set(typeName, types);
    if (isInaccessible(definition)) inaccessibleTypes.add(typeName);
    for (const { name } of directiveArgs(definition, 'context')) {
      if (!contextTypes.has(name)) contextTypes.set(name, new Set());
      contextTypes.get(name).add(typeName);
    }
    if (definition.kind === Kind.OBJECT_TYPE_DEFINITION && types.some((type) => type.key)) {
      entityTypes.push(typeName);
    }
    for (const args of directiveArgs(definition, 'join__implements')) {
      if (!implementsIn.has(args.interface)) implementsIn.set(args.interface, []);
      implementsIn.get(args.interface).push({ graph: graph(args.graph), type: typeName });
    }
    if (definition.kind === Kind.UNION_TYPE_DEFINITION) {
      unionMembers.set(
        typeName,
        directiveArgs(definition, 'join__unionMember').map((args) => ({ ...args, graph: graph(args.graph) })),
      );
    }
    for (const field of definition.fields ?? []) {
      const coordinate = `${typeName}.${field.name.value}`;
      const entries = directiveArgs(field, 'join__field').map((args) => ({
        ...args,
        graph: args.graph && graph(args.graph),
      }));
      joinFields.set(coordinate, entries);
      const fromContext = entries.flatMap((entry) => entry.contextArguments ?? []);
      if (fromContext.length) contextArguments.set(coordinate, fromContext);
    }
  }

  // The router passes a `@fromContext` value as an argument the supergraph doesn't declare. It's
  // added here, nullable, so the monolith can leave it out and work it out itself.
  const definitions = document.definitions.map((definition) => {
    if (!definition.fields || definition.kind === Kind.INPUT_OBJECT_TYPE_DEFINITION) return definition;
    const typeName = definition.name.value;
    if (!definition.fields.some((field) => contextArguments.has(`${typeName}.${field.name.value}`))) {
      return definition;
    }
    return {
      ...definition,
      fields: definition.fields.map((field) => {
        const fromContext = contextArguments.get(`${typeName}.${field.name.value}`);
        if (!fromContext) return field;
        const added = fromContext
          .filter((argument) => !field.arguments?.some((existing) => existing.name.value === argument.name))
          .map((argument) => {
            const type = parseType(argument.type);
            return {
              kind: Kind.INPUT_VALUE_DEFINITION,
              name: { kind: Kind.NAME, value: argument.name },
              type: type.kind === Kind.NON_NULL_TYPE ? type.type : type,
            };
          });
        return { ...field, arguments: [...(field.arguments ?? []), ...added] };
      }),
    };
  });

  // a supergraph without entities has no `_entities` either. It goes on the query root, whatever
  // it's called (`schema { query: RootQuery }`)
  const queryRoot =
    document.definitions
      .find((definition) => definition.kind === Kind.SCHEMA_DEFINITION)
      ?.operationTypes.find((operationType) => operationType.operation === 'query')?.type.name.value ?? 'Query';
  const extended = entityTypes.length
    ? parse(`
        scalar _Any
        union _Entity = ${entityTypes.join(' | ')}
        extend type ${queryRoot} { _entities(representations: [_Any!]!): [_Entity]! }
      `)
    : { definitions: [] };
  const schema = buildASTSchema(
    { kind: Kind.DOCUMENT, definitions: [...definitions, ...extended.definitions] },
    // graphql 17 rejects deprecated implementations of non-deprecated interface fields, which a
    // composed supergraph has plenty of
    { assumeValidSDL: true, assumeValid: true },
  );

  const ownersCache = new Map();
  /** The subgraphs that can resolve `typeName.fieldName` themselves. */
  function owners(typeName, fieldName) {
    const coordinate = `${typeName}.${fieldName}`;
    let result = ownersCache.get(coordinate);
    if (result) return result;
    const fields = joinFields.get(coordinate) ?? [];
    const typeGraphs = (joinTypes.get(typeName) ?? []).map((type) => type.graph);
    if (fields.length === 0 || fields.some((field) => !field.graph)) {
      result = new Set(typeGraphs);
    } else {
      result = new Set(
        fields.filter((field) => !field.external && !field.usedOverridden).map((field) => field.graph),
      );
    }
    ownersCache.set(coordinate, result);
    return result;
  }

  const interfacesOf = new Map();
  for (const [interfaceName, entries] of implementsIn) {
    for (const { graph: graphName, type } of entries) {
      if (!interfacesOf.has(type)) interfacesOf.set(type, []);
      interfacesOf.get(type).push({ graph: graphName, interface: interfaceName });
    }
  }

  /**
   * Whether subgraph `graphName` resolves `typeName.fieldName`. Through an interface the type
   * implements there counts too: WFG has types that implement an interface in a subgraph without
   * a `join__field` for that subgraph on the interface's fields.
   */
  function resolves(typeName, fieldName, graphName) {
    if (owners(typeName, fieldName).has(graphName)) return true;
    if (
      (interfacesOf.get(typeName) ?? []).some(
        (entry) => entry.graph === graphName && owners(entry.interface, fieldName).has(graphName),
      )
    ) {
      return true;
    }
    // an `@interfaceObject` subgraph has the interface as an object, and knows nothing of the
    // types behind it
    const type = schema.getType(typeName);
    return (type?.getInterfaces?.() ?? []).some(
      (implemented) =>
        isInterfaceObjectIn(implemented.name, graphName) &&
        owners(implemented.name, fieldName).has(graphName),
    );
  }

  function isInterfaceObjectIn(typeName, graphName) {
    return (joinTypes.get(typeName) ?? []).some(
      (type) => type.graph === graphName && type.isInterfaceObject,
    );
  }

  /** The object types of `abstractName` that subgraph `graphName` knows. */
  function possibleTypesIn(abstractName, graphName) {
    const abstractType = schema.getType(abstractName);
    if (isUnionType(abstractType)) {
      return (unionMembers.get(abstractName) ?? [])
        .filter((member) => member.graph === graphName)
        .map((member) => member.member);
    }
    if (isInterfaceType(abstractType)) {
      return (implementsIn.get(abstractName) ?? [])
        .filter((entry) => entry.graph === graphName)
        .map((entry) => entry.type);
    }
    return [abstractName];
  }

  let providedBy = null;
  /** The subgraphs that `@provides` `typeName.fieldName` somewhere. */
  function providers(typeName, fieldName) {
    if (!providedBy) {
      providedBy = new Map();
      const walk = (selectionSet, parentName, graphName) => {
        for (const selection of selectionSet.selections) {
          if (selection.kind === Kind.INLINE_FRAGMENT) {
            walk(selection.selectionSet, selection.typeCondition?.name.value ?? parentName, graphName);
            continue;
          }
          const coordinate = `${parentName}.${selection.name.value}`;
          if (!providedBy.has(coordinate)) providedBy.set(coordinate, new Set());
          providedBy.get(coordinate).add(graphName);
          const field = schema.getType(parentName)?.getFields?.()[selection.name.value];
          if (field && selection.selectionSet) {
            walk(selection.selectionSet, getNamedType(field.type).name, graphName);
          }
        }
      };
      for (const [coordinate, entries] of joinFields) {
        const [parentName, name] = coordinate.split('.');
        const field = schema.getType(parentName)?.getFields?.()[name];
        if (!field) continue;
        for (const entry of entries) {
          if (!entry.provides || !entry.graph) continue;
          walk(parse(`{ ${entry.provides} }`).definitions[0].selectionSet, getNamedType(field.type).name, entry.graph);
        }
      }
    }
    return providedBy.get(`${typeName}.${fieldName}`) ?? [];
  }

  const candidatesCache = new Map();
  /**
   * The object types `typeName.fieldName` can return. Every subgraph that resolves the field has to
   * know the type, so that whichever one the router asks gives the same answer. Empty when there's
   * no such type, a value type shared by many subgraphs often has an interface field like that.
   */
  function concreteCandidates(typeName, fieldName, abstractName) {
    const cacheKey = `${typeName}.${fieldName}`;
    let result = candidatesCache.get(cacheKey);
    if (result) return result;
    const abstractType = schema.getType(abstractName);
    // a type the client can't see would come back as one the router doesn't know
    const all = schema
      .getPossibleTypes(abstractType)
      .map((type) => type.name)
      .filter((name) => !isAbstractType(schema.getType(name)) && !inaccessibleTypes.has(name));
    let shared = null;
    for (const graphName of [...owners(typeName, fieldName), ...providers(typeName, fieldName)]) {
      const known = new Set(possibleTypesIn(abstractName, graphName));
      shared = shared ? shared.filter((name) => known.has(name)) : all.filter((name) => known.has(name));
    }
    result = (shared ?? all).sort();
    candidatesCache.set(cacheKey, result);
    return result;
  }

  const keyTreeCache = new Map();
  /** Every key of the type merged into one tree, `null` when it isn't an entity. */
  function keyTree(typeName) {
    if (keyTreeCache.has(typeName)) return keyTreeCache.get(typeName);
    const keys = (joinTypes.get(typeName) ?? []).filter((type) => type.key).map((type) => fieldSetTree(type.key));
    const tree = keys.length ? keys.reduce(mergeTrees) : null;
    keyTreeCache.set(typeName, tree);
    return tree;
  }

  const providesCache = new Map();
  /** What `coordinate` provides in subgraph `graphName`, as a tree, `null` for nothing. */
  function provides(coordinate, graphName) {
    const cacheKey = `${coordinate}|${graphName}`;
    if (providesCache.has(cacheKey)) return providesCache.get(cacheKey);
    const entry = (joinFields.get(coordinate) ?? []).find((field) => field.graph === graphName && field.provides);
    const tree = entry ? fieldSetTree(entry.provides) : null;
    providesCache.set(cacheKey, tree);
    return tree;
  }

  const requiresCache = new Map();
  /** What `typeName.fieldName` `@requires`, as a selection set, `null` for nothing. */
  function requires(typeName, fieldName) {
    const coordinate = `${typeName}.${fieldName}`;
    if (requiresCache.has(coordinate)) return requiresCache.get(coordinate);
    const selections = (joinFields.get(coordinate) ?? [])
      .filter((field) => field.requires)
      .flatMap((field) => parse(`{ ${field.requires} }`).definitions[0].selectionSet.selections);
    const selectionSet = selections.length ? { kind: Kind.SELECTION_SET, selections } : null;
    requiresCache.set(coordinate, selectionSet);
    return selectionSet;
  }

  let requiredFieldNodes = null;
  /**
   * Every way a `@requires` asks for a field called `fieldName`, as field nodes, for the
   * arguments a representation's value was fetched with.
   */
  function requiredFields(fieldName) {
    if (!requiredFieldNodes) {
      requiredFieldNodes = new Map();
      const walk = (selectionSet) => {
        for (const selection of selectionSet.selections) {
          if (selection.kind === Kind.FIELD) {
            const name = selection.name.value;
            if (!requiredFieldNodes.has(name)) requiredFieldNodes.set(name, []);
            requiredFieldNodes.get(name).push(selection);
          }
          if (selection.selectionSet) walk(selection.selectionSet);
        }
      };
      for (const coordinate of joinFields.keys()) {
        const [typeName, name] = coordinate.split('.');
        const selectionSet = requires(typeName, name);
        if (selectionSet) walk(selectionSet);
      }
    }
    return requiredFieldNodes.get(fieldName) ?? [];
  }

  let contextFieldNames = null;
  /** Whether the router may fetch a field called `fieldName` as a `@requires` or context input. */
  function inputField(fieldName) {
    if (!contextFieldNames) {
      contextFieldNames = new Set();
      const walk = (selectionSet) => {
        for (const selection of selectionSet.selections) {
          if (selection.kind === Kind.FIELD) contextFieldNames.add(selection.name.value);
          if (selection.selectionSet) walk(selection.selectionSet);
        }
      };
      for (const entries of contextArguments.values()) {
        for (const { selection } of entries) {
          const text = selection.trim();
          walk(parse(text.startsWith('{') ? `query ${text}` : `{ ${text} }`).definitions[0].selectionSet);
        }
      }
    }
    return contextFieldNames.has(fieldName) || requiredFields(fieldName).length > 0;
  }

  /**
   * Whether a `@requires` of `typeName` asks for a field called `fieldName` at `path` (field names
   * from the entity) or below it.
   */
  function requiresUnder(typeName, path, fieldName) {
    const walk = (selectionSet, names) =>
      selectionSet.selections.some((selection) => {
        const next = selection.kind === Kind.FIELD ? [...names, selection.name.value] : names;
        if (
          selection.kind === Kind.FIELD &&
          selection.name.value === fieldName &&
          path.every((step, index) => next[index] === step)
        ) {
          return true;
        }
        return selection.selectionSet ? walk(selection.selectionSet, next) : false;
      });
    for (const coordinate of joinFields.keys()) {
      const [owner, name] = coordinate.split('.');
      if (owner !== typeName) continue;
      const selectionSet = requires(typeName, name);
      if (selectionSet && walk(selectionSet, [])) return true;
    }
    return false;
  }

  /** Whether subgraph `graphName` can take `typeName` in `_entities`. */
  function resolvesEntity(typeName, graphName) {
    return (joinTypes.get(typeName) ?? []).some(
      (type) => type.graph === graphName && type.key && type.resolvable !== false,
    );
  }

  /**
   * Whether `representation` has every field of one key `graphName` resolves `typeName` by. A key
   * of another subgraph doesn't count, the receiving one can't look the entity up with it.
   */
  function hasKeyIn(typeName, graphName, representation) {
    // a value with subfields has to be an object, or a list of them. Anything else, like a
    // string where `category { id }` goes, isn't a key
    const fits = (tree, value) => {
      if (value == null) return true;
      if (Array.isArray(value)) return value.every((item) => fits(tree, item));
      return typeof value === 'object' && covers(tree, value);
    };
    const covers = (tree, value) =>
      Object.entries(tree).every(([name, subtree]) => name in value && (!subtree || fits(subtree, value[name])));
    return (joinTypes.get(typeName) ?? [])
      .filter((type) => type.graph === graphName && type.key && type.resolvable !== false)
      .some((type) => covers(fieldSetTree(type.key), representation));
  }

  const urls = Object.values(graphNames);
  return {
    schema,
    owners,
    resolves,
    concreteCandidates,
    keyTree,
    resolvesEntity,
    hasKeyIn,
    graphNames: urls,
    contextArguments: (typeName, fieldName) => contextArguments.get(`${typeName}.${fieldName}`),
    contextTypes: (name) => contextTypes.get(name) ?? new Set(),
    isInterfaceObjectIn,
    provides,
    requires,
    requiredFields,
    inputField,
    requiresUnder,
  };
}

/** The supergraph with every subgraph URL pointing at `baseUrl/<name>`. */
export function withSubgraphUrls(sdl, baseUrl) {
  return sdl.replace(
    /@join__graph\(name: "([^"]+)", url: "[^"]*"\)/g,
    (_, name) => `@join__graph(name: "${name}", url: "${baseUrl}/${name}")`,
  );
}

const NOT_UNIQUE = new Set(['Int', 'Float', 'Boolean']);

/**
 * The supergraph with every `Int`, `Float` and `Boolean` a field returns turned into a `String`, so
 * those values are unique too and the `unused` check sees them. Only what fields return,
 * arguments and input fields keep their types, the operations' variables and literals need them.
 * `{ sdl, changed }`, `changed` is how many fields.
 */
export function stringifyScalars(sdl) {
  let changed = 0;
  const retype = (type) => {
    if (type.kind !== Kind.NAMED_TYPE) return { ...type, type: retype(type.type) };
    if (!NOT_UNIQUE.has(type.name.value)) return type;
    return { ...type, name: { ...type.name, value: 'String' } };
  };
  const document = visit(parse(sdl), {
    FieldDefinition(field) {
      const type = retype(field.type);
      if (type === field.type || print(type) === print(field.type)) return undefined;
      changed++;
      return { ...field, type };
    },
  });
  return { sdl: print(document), changed };
}
