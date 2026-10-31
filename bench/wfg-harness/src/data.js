// The data every subgraph serves, and the monolith too. A value is a pure function of where it
// lives, so subgraphs agree on it without sharing anything:
//
// - an object is `{ __typename, seed }`, a child's seed comes from its parent's seed, the field,
//   the arguments and the list index
// - an entity's key fields carry its seed (`Type:seed`, or the seed itself for an `Int`), so a
//   subgraph gets the same entity back from a representation
// - a `String`/`ID` that isn't a key is `Parent.field:hash`, unique, so the runner can tell where
//   every value the router fetched ended up
//
// The seed keeps the depth in its low bits, lists get 2 items near the root and 1 below, so a
// big operation doesn't blow up. A list of an abstract type gets an item of every type it can be
// instead, so every type condition in the operation has data to go through.

import {
  getArgumentValues,
  getNamedType,
  isAbstractType,
  isEnumType,
  isLeafType,
  isListType,
  isNonNullType,
  isObjectType,
  Kind,
  parse,
} from 'graphql';

import { isInaccessible } from './supergraph.js';

/** What a field that fails returns, the resolver throws on it. */
export const FIELD_ERROR = Symbol('field error');

/** cyrb53, 53 bits */
export function hash(text) {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < text.length; i++) {
    const ch = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

const DEPTH_MASK = 7;
// fits a GraphQL `Int`, which an `Int` key has to
const makeSeed = (h, depth) => ((h % 0x80000000) & ~DEPTH_MASK) | Math.min(depth, DEPTH_MASK);
const depthOf = (seed) => seed & DEPTH_MASK;
const listLength = (seed) => (depthOf(seed) < 2 ? 2 : 1);
const MAX_TYPES_PER_LIST = 64;

export function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(',')}]`;
  if (value && typeof value === 'object') {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
      .join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

export function createData(supergraph) {
  const { schema, concreteCandidates, keyTree, contextArguments, contextTypes } = supergraph;
  // The object types the operation has type conditions on, and which case this is. A single
  // abstract value picks among the preferred types, rotating per case, so the cases between them
  // go down every branch the operation has. With `faults`, some nullable fields are null and some
  // leaf fields fail, the same ones for the router's subgraphs and the monolith.
  const variant = { preferredTypes: new Set(), salt: 0, faults: false };

  function pick(candidates, h) {
    const preferred = candidates.filter((name) => variant.preferredTypes.has(name));
    const pool = preferred.length ? preferred : candidates;
    return pool[(h + variant.salt) % pool.length];
  }

  function entityRef(typeName, seed) {
    return { __typename: typeName, seed, entity: { type: typeName, seed }, keyFields: keyTree(typeName) };
  }

  function objectRef(typeName, seed) {
    return keyTree(typeName) ? entityRef(typeName, seed) : { __typename: typeName, seed };
  }

  // --- `@fromContext`

  const selections = new Map();
  function parseSelection(selection) {
    let parsed = selections.get(selection);
    if (!parsed) {
      const text = selection.trim().startsWith('{') ? selection : `{ ${selection} }`;
      parsed = parse(text).definitions[0].selectionSet;
      selections.set(selection, parsed);
    }
    return parsed;
  }

  function isType(typeName, condition) {
    if (typeName === condition) return true;
    const abstractType = schema.getType(condition);
    return isAbstractType(abstractType) && schema.isSubType(abstractType, schema.getType(typeName));
  }

  /** A context set on an interface or a union is set by every type behind it. */
  function setsContext(ref, types) {
    for (const type of types) if (isType(ref.__typename, type)) return true;
    return false;
  }

  /** What the router reads for a `@fromContext` argument: `selection` of the nearest object that sets the context. */
  function contextValue(ref, { context, selection }) {
    const types = contextTypes(context);
    let holder = ref;
    while (holder && !setsContext(holder, types)) holder = holder.parent;
    if (!holder) return null;
    const read = (object, selectionSet) => {
      for (const selection of selectionSet.selections) {
        if (selection.kind === Kind.FIELD) {
          if (selection.name.value === '__typename') return object.__typename;
          const field = schema.getType(object.__typename).getFields()[selection.name.value];
          if (!field) continue;
          const value = fieldValue(object, object.__typename, field.name, field.type, {});
          if (value === FIELD_ERROR || value == null) return null;
          return selection.selectionSet && typeof value === 'object' && !Array.isArray(value)
            ? read(value, selection.selectionSet)
            : value;
        }
        if (
          selection.kind === Kind.INLINE_FRAGMENT &&
          (!selection.typeCondition || isType(object.__typename, selection.typeCondition.name.value))
        ) {
          const value = read(object, selection.selectionSet);
          if (value !== undefined) return value;
        }
      }
      return undefined;
    };
    return read(holder, parseSelection(selection)) ?? null;
  }

  function keyLeaf(entity, fieldName, typeName) {
    if (typeName === 'Int' || typeName === 'Float') return entity.seed;
    if (typeName === 'Boolean') return hash(`${entity.seed}|${fieldName}`) % 2 === 0;
    return `${entity.type}:${entity.seed}`;
  }

  function plainLeaf(type, parentName, fieldName, h) {
    if (isEnumType(type)) {
      const values = type.getValues().filter((value) => !isInaccessible(value.astNode));
      return values[h % values.length].name;
    }
    switch (type.name) {
      case 'Int':
        return h % 1000;
      case 'Float':
        return (h % 1000) / 4;
      case 'Boolean':
        return h % 2 === 0;
      default:
        return `${parentName}.${fieldName}:${h.toString(36)}`;
    }
  }

  /** `parentName.fieldName(args)` of `ref`, objects come back as refs. */
  function fieldValue(ref, parentName, fieldName, returnType, args) {
    // the router passes `@fromContext` values to subgraphs, the monolith works them out, and the
    // value depends on them, so a wrong one shows in the data
    const fromContext = contextArguments(parentName, fieldName);
    if (fromContext) {
      args = { ...args };
      for (const argument of fromContext) {
        if (args[argument.name] === undefined) args[argument.name] = contextValue(ref, argument);
      }
    }
    // a field that `@requires` others depends on them too: a subgraph reads them from the
    // representation, the monolith from the entity. A wrong one shows in the data, one nobody
    // reads doesn't matter
    const required = supergraph.requires(parentName, fieldName);
    if (required) {
      args = { ...args, '@requires': ref.representation ? fromRepresentation(ref.representation, required) : project(ref, required) };
    }
    // a key field comes from the entity alone, a representation has nothing else
    const inKey = ref.keyFields && fieldName in ref.keyFields;
    const h = inKey
      ? hash(`${ref.entity.type}|${ref.entity.seed}|${parentName}|${fieldName}`)
      : hash(`${ref.seed}|${fieldName}|${stableJson(args ?? {})}`);
    if (variant.faults && !inKey) {
      if (!isNonNullType(returnType) && h % 13 === 5) return null;
      if (isLeafType(getNamedType(returnType)) && h % 29 === 7) return FIELD_ERROR;
    }
    const declaredType = getNamedType(returnType).name;
    const build = (type, h, nullable = true, index = null) => {
      if (isNonNullType(type)) return build(type.ofType, h, false, index);
      if (isListType(type)) {
        // one item of every type it can be, so every type condition gets something
        const item = getNamedType(type);
        const length = isAbstractType(item)
          ? Math.min(Math.max(concreteCandidates(parentName, fieldName, item.name).length, 1), MAX_TYPES_PER_LIST)
          : listLength(ref.seed);
        return Array.from({ length }, (_, index) => build(type.ofType, hash(`${h}|${index}`), true, index));
      }
      if (isLeafType(type)) {
        return inKey && !isEnumType(type)
          ? keyLeaf(ref.entity, fieldName, type.name)
          : plainLeaf(type, parentName, fieldName, h);
      }
      let typeName = type.name;
      if (!isObjectType(type)) {
        const candidates = concreteCandidates(parentName, fieldName, type.name);
        if (candidates.length === 0) {
          // no type every subgraph resolving it knows, null is the one answer they all agree on
          if (nullable) return null;
          typeName = pick(schema.getPossibleTypes(type).map((possible) => possible.name).sort(), h);
        } else {
          typeName = index == null ? pick(candidates, h) : candidates[index % candidates.length];
        }
      }
      // a field that returns `Query` returns the root, the router asks the root for it
      if (typeName === schema.getQueryType()?.name) return { __typename: typeName, seed: 0 };
      const seed = makeSeed(h, depthOf(ref.seed) + 1);
      if (inKey) {
        // a key like `searchCriteria { partialCriteria }` on an entity that's one itself is that
        // entity, under the same seed, so either key reads the same
        if (keyTree(typeName)) return { ...entityRef(typeName, ref.entity.seed), parent: ref, declaredType };
        // a key like `field768 { field19 }`, its leaves carry the entity's seed too
        return { __typename: typeName, seed, entity: ref.entity, keyFields: ref.keyFields[fieldName] ?? {} };
      }
      // the parent is for `@fromContext`, an entity from a representation has none, but the
      // router passes those values then. `via` is the field it came from, for `@provides`
      return { ...objectRef(typeName, seed), parent: ref, declaredType, via: `${parentName}.${fieldName}` };
    };
    return build(returnType, h);
  }

  /**
   * The arguments a resolver gets for `field` asked for as `node`, defaults included. `null` when
   * that can't be asked, a required argument is missing.
   */
  function argumentsOf(field, node) {
    const asked = node ?? { kind: Kind.FIELD, name: { kind: Kind.NAME, value: field.name }, arguments: [] };
    try {
      return { ...getArgumentValues(field, asked, undefined) };
    } catch {
      return null;
    }
  }

  function applies(typeName, selection) {
    return !selection.typeCondition || isType(typeName, selection.typeCondition.name.value);
  }

  /** What `ref` has for `selectionSet` (a `@requires`), as a representation would have it. */
  function project(ref, selectionSet, out = {}) {
    const type = schema.getType(ref.__typename);
    for (const selection of selectionSet.selections) {
      if (selection.kind === Kind.INLINE_FRAGMENT) {
        if (applies(ref.__typename, selection)) project(ref, selection.selectionSet, out);
        continue;
      }
      if (selection.name.value === '__typename') {
        out.__typename = ref.__typename;
        continue;
      }
      const field = type.getFields()[selection.name.value];
      if (!field) continue;
      const value = fieldValue(ref, ref.__typename, field.name, field.type, argumentsOf(field, selection) ?? {});
      const shape = (value) => {
        if (value === FIELD_ERROR || value == null) return null;
        if (Array.isArray(value)) return value.map(shape);
        return selection.selectionSet && typeof value === 'object' ? project(value, selection.selectionSet) : value;
      };
      out[field.name] = shape(value);
    }
    return out;
  }

  /**
   * What a representation has for `selectionSet` (a `@requires`). One it doesn't have is left
   * out, like `project` leaves out a field the type doesn't have, so one the router forgot still
   * differs.
   */
  function fromRepresentation(representation, selectionSet, out = {}) {
    for (const selection of selectionSet.selections) {
      if (selection.kind === Kind.INLINE_FRAGMENT) {
        fromRepresentation(representation, selection.selectionSet, out);
        continue;
      }
      const name = selection.name.value;
      if (!(name in representation)) continue;
      const shape = (value) => {
        if (value == null) return null;
        if (Array.isArray(value)) return value.map(shape);
        return selection.selectionSet && typeof value === 'object'
          ? fromRepresentation(value, selection.selectionSet)
          : value;
      };
      out[name] = shape(representation[name]);
    }
    return out;
  }

  /**
   * The entity a representation stands for, from the seed in any of its key fields. A key names
   * the object type too, which is how an `@interfaceObject` or entity interface representation
   * (`__typename: "Account"`) gets back to the `User` it is.
   */
  function decodeEntity(typeName, representation) {
    const tree = keyTree(typeName);
    if (!tree) return null;
    let objectType = isObjectType(schema.getType(typeName)) ? typeName : null;
    const find = (value) => {
      if (typeof value === 'number' && Number.isInteger(value)) return value;
      if (typeof value === 'string') {
        const match = /^([A-Za-z0-9_]+):(\d+)$/.exec(value);
        if (!match || !isObjectType(schema.getType(match[1]))) return null;
        objectType ??= match[1];
        return Number(match[2]);
      }
      if (Array.isArray(value)) return value.map(find).find((seed) => seed != null) ?? null;
      if (value && typeof value === 'object') {
        return Object.entries(value)
          .filter(([key]) => key !== '__typename')
          .map(([, inner]) => find(inner))
          .find((seed) => seed != null) ?? null;
      }
      return null;
    };
    for (const fieldName of Object.keys(tree)) {
      if (fieldName in representation) {
        const seed = find(representation[fieldName]);
        if (seed != null && objectType) return { ...entityRef(objectType, seed), representation };
      }
    }
    return null;
  }

  /**
   * Why a representation didn't decode, by key field: the ones `missing`, `malformed` values in
   * them (neither a key, a seed, nor a value of this data) and where their `nulls` are, as paths.
   */
  function whyUndecodable(typeName, representation) {
    const causes = { typeName, missing: [], malformed: [], nulls: [] };
    const walk = (value, path) => {
      if (value === null) causes.nulls.push(path);
      else if (Array.isArray(value)) value.forEach((item) => walk(item, path));
      else if (typeof value === 'object') {
        for (const [key, inner] of Object.entries(value)) if (key !== '__typename') walk(inner, [...path, key]);
      } else if (!Number.isInteger(value) && !(typeof value === 'string' && /^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+:[0-9a-z]+|:\d+)$/.test(value))) {
        causes.malformed.push(path);
      }
    };
    for (const fieldName of Object.keys(keyTree(typeName) ?? {})) {
      if (!(fieldName in representation)) causes.missing.push([fieldName]);
      else walk(representation[fieldName], [fieldName]);
    }
    return causes;
  }

  /**
   * Every field of a representation has to be what the entity really has, keys and `@requires`
   * fields alike. A null one is fine, the router sends that for what it didn't fetch, and when a
   * field reads it the data shows it. Returns the differences.
   */
  function verifyRepresentation(ref, representation) {
    const differences = [];
    const compare = (expected, actual, path) => {
      if (Array.isArray(expected)) {
        if (actual === null) return;
        if (!Array.isArray(actual) || actual.length !== expected.length) {
          differences.push(`${path}: expected a list of ${expected.length}`);
          return;
        }
        expected.forEach((item, index) => compare(item, actual[index], `${path}.${index}`));
      } else if (expected && typeof expected === 'object') {
        if (actual === null) return;
        if (typeof actual !== 'object') {
          differences.push(`${path}: expected an object`);
          return;
        }
        verifyObject(expected, actual, path);
      } else if (actual !== null && (expected === FIELD_ERROR ? null : expected) !== actual) {
        differences.push(`${path}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
    };
    const verifyObject = (ref, actual, path) => {
      const type = schema.getType(ref.__typename);
      for (const [fieldName, value] of Object.entries(actual)) {
        if (fieldName === '__typename') {
          // an interface the type implements is fine, `@interfaceObject`s are sent like that
          if (!isType(ref.__typename, value)) {
            differences.push(`${path}.__typename: expected ${ref.__typename}, got ${value}`);
          }
          continue;
        }
        const field = type.getFields()[fieldName];
        if (!field) {
          differences.push(`${path}.${fieldName}: no such field`);
          continue;
        }
        // a `@requires` may have asked for it with arguments, any of them will do
        const asked = [null, ...supergraph.requiredFields(fieldName)];
        let first = null;
        for (const node of asked) {
          const args = argumentsOf(field, node);
          if (!args) continue;
          const before = differences.length;
          compare(fieldValue(ref, ref.__typename, fieldName, field.type, args), value, `${path}.${fieldName}`);
          const found = differences.splice(before);
          if (!found.length) {
            first = null;
            break;
          }
          first ??= found;
        }
        if (first) differences.push(...first);
      }
    };
    // what the entity really has, not what this representation says it has
    verifyObject({ ...ref, representation: undefined }, representation, ref.__typename);
    return differences;
  }

  return { variant, fieldValue, project, decodeEntity, whyUndecodable, verifyRepresentation };
}

