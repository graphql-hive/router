#!/usr/bin/env node

import { writeFileSync } from 'node:fs';
import { basename } from 'node:path';

const DEFAULT_SIZE = '3mb';
const DEFAULT_OUT = 'ct-payload.json';
const PRODUCT_COUNT = 90;
const EXPANDABLE_STRING_KEYS = new Set([
  'city',
  'description',
  'firstName',
  'label',
  'lastName',
  'name',
  'predicate',
  'region',
  'state',
  'streetName',
  'text',
  'value',
]);
const FILLER_WORDS = [
  'catalog',
  'localized',
  'variant',
  'channel',
  'attribute',
  'discount',
  'product',
  'search',
  'category',
  'generated',
  'payload',
  'sample',
];

function parseArgs(argv) {
  const args = {
    out: DEFAULT_OUT,
    size: DEFAULT_SIZE,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];

    if (arg === '--out' || arg === '-o') {
      args.out = argv[++i];
    } else if (arg === '--size' || arg === '-s') {
      args.size = argv[++i];
    } else if (arg === '--help' || arg === '-h') {
      printHelp();
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!args.out) {
    throw new Error('Missing value for --out');
  }

  if (!args.size) {
    throw new Error('Missing value for --size');
  }

  return args;
}

function parseSize(value) {
  const match = String(value)
    .trim()
    .toLowerCase()
    .match(/^(\d+(?:\.\d+)?)(b|kb|kib|mb|mib)?$/);

  if (!match) {
    throw new Error(`Invalid size: ${value}. Use bytes, kb/kib, or mb/mib, for example 3145728 or 3mb.`);
  }

  const amount = Number(match[1]);
  const unit = match[2] ?? 'b';
  const multiplier = {
    b: 1,
    kb: 1000,
    kib: 1024,
    mb: 1000 * 1000,
    mib: 1024 * 1024,
  }[unit];

  return Math.floor(amount * multiplier);
}

function localized(value, locale = 'en-US') {
  return { locale, value };
}

function makeCategory(index, suffix) {
  return {
    nameAllLocales: [localized(`Category ${index} ${suffix}`), localized(`Kategorie ${index} ${suffix}`, 'de-DE')],
    descriptionAllLocales: [localized(`Category description ${index} ${suffix}`)],
    slugAllLocales: [localized(`category-${index}-${suffix.toLowerCase()}`)],
    childCount: 2,
  };
}

function makeAddress(index) {
  return {
    streetName: `Main Street ${index}`,
    streetNumber: `${index}`,
    postalCode: `10${String(index).padStart(3, '0')}`,
    city: 'Berlin',
    region: 'Berlin',
    state: 'BE',
    firstName: `First${index}`,
    lastName: `Last${index}`,
  };
}

function makePrice(productIndex, variantIndex, priceIndex) {
  const amount = 1299 + productIndex + variantIndex + priceIndex;

  return {
    id: `price-${productIndex}-${variantIndex}-${priceIndex}`,
    value: {
      type: 'centPrecision',
      centAmount: amount,
      currencyCode: 'EUR',
    },
    customerGroup: {
      name: `Customer group ${priceIndex}`,
      key: `customer-group-${priceIndex}`,
    },
    channel: {
      id: `channel-${priceIndex}`,
      nameAllLocales: [localized(`Channel ${priceIndex}`), localized(`Kanal ${priceIndex}`, 'de-DE')],
      descriptionAllLocales: [localized(`Channel description ${priceIndex}`)],
      address: makeAddress(priceIndex),
    },
    discounted: {
      value: {
        currencyCode: 'EUR',
        centAmount: amount - 100,
      },
      discount: {
        predicate: 'sku exists',
        validFrom: '2026-01-01T00:00:00.000Z',
        validUntil: '2026-12-31T23:59:59.999Z',
        key: `discount-${priceIndex}`,
        nameAllLocales: [localized(`Discount ${priceIndex}`), localized(`Rabatt ${priceIndex}`, 'de-DE')],
      },
    },
  };
}

function makeVariant(productIndex, variantIndex) {
  return {
    id: variantIndex,
    key: `variant-${productIndex}-${variantIndex}`,
    prices: [makePrice(productIndex, variantIndex, 1), makePrice(productIndex, variantIndex, 2)],
    attributesRaw: [
      {
        name: 'material',
        value: {
          label: 'cotton',
          score: productIndex + variantIndex,
          active: true,
        },
      },
      {
        name: 'tags',
        value: ['summer', 'sale', `product-${productIndex}`],
      },
    ],
  };
}

function makeProduct(index) {
  const category = makeCategory(index, 'Root');

  return {
    id: `product-${index}`,
    skus: [`sku-${index}-1`, `sku-${index}-2`],
    state: {
      id: `state-${index}`,
      key: 'published',
      type: 'ProductState',
      roles: ['ReviewIncludedInStatistics'],
      nameAllLocales: [localized('Published'), localized('Veroeffentlicht', 'de-DE')],
      descriptionAllLocales: [localized('Product is published')],
    },
    priceMode: 'Embedded',
    taxCategory: {
      name: 'Standard tax',
      description: 'Standard German VAT category',
      key: 'standard-tax',
      id: 'tax-category-standard',
      rates: [
        {
          name: 'DE standard',
          amount: 0.19,
          country: 'DE',
          state: null,
        },
      ],
    },
    productType: {
      key: 'sample-product-type',
      name: 'Sample Product Type',
      description: `Generated product type ${index}`,
      attributeDefinitions: {
        results: [
          {
            type: { name: 'text' },
            name: 'material',
            attributeConstraint: 'None',
            labelAllLocales: [localized('Material'), localized('Material', 'de-DE')],
          },
          {
            type: { name: 'set' },
            name: 'tags',
            attributeConstraint: 'None',
            labelAllLocales: [localized('Tags')],
          },
        ],
      },
    },
    masterData: {
      current: {
        metaTitleAllLocales: [localized(`Product ${index}`)],
        metaKeywordsAllLocales: [localized(`product,generated,${index}`)],
        metaDescriptionAllLocales: [localized(`Generated product ${index}`)],
        categories: [
          {
            ...category,
            children: [makeCategory(index, 'Child A'), makeCategory(index, 'Child B')],
            ancestors: [makeCategory(index, 'Ancestor')],
          },
        ],
        searchKeywords: [
          {
            locale: 'en-US',
            searchKeywords: [{ text: `product ${index}` }, { text: `generated ${index}` }],
          },
        ],
        allVariants: [makeVariant(index, 1), makeVariant(index, 2)],
      },
    },
  };
}

function makePayload() {
  return {
    data: {
      products: {
        results: Array.from({ length: PRODUCT_COUNT }, (_, index) => makeProduct(index + 1)),
      },
    },
  };
}

function stringify(payload) {
  return JSON.stringify(payload);
}

function byteLength(value) {
  return Buffer.byteLength(value, 'utf8');
}

function collectExpandableStrings(value, slots = []) {
  if (Array.isArray(value)) {
    for (const item of value) {
      collectExpandableStrings(item, slots);
    }
  } else if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) {
      if (EXPANDABLE_STRING_KEYS.has(key) && typeof child === 'string') {
        slots.push({ object: value, key });
      } else {
        collectExpandableStrings(child, slots);
      }
    }
  }

  return slots;
}

function makeFiller(length, seed) {
  if (length === 0) {
    return '';
  }

  let filler = '';
  let cursor = seed;

  while (filler.length < length) {
    const word = FILLER_WORDS[cursor % FILLER_WORDS.length];
    const next = filler.length === 0 ? ` ${word}` : ` ${word}`;
    filler += next;
    cursor++;
  }

  return filler.slice(0, length);
}

function resizePayload(payload, targetBytes) {
  const slots = collectExpandableStrings(payload);

  const baseJson = stringify(payload);
  const baseBytes = byteLength(baseJson);

  if (baseBytes > targetBytes) {
    throw new Error(
      `Requested size ${targetBytes} bytes is too small. Minimum for this payload shape is ${baseBytes} bytes.`,
    );
  }

  const extraBytes = targetBytes - baseBytes;
  const bytesPerSlot = Math.floor(extraBytes / slots.length);
  const remainder = extraBytes % slots.length;

  for (let i = 0; i < slots.length; i++) {
    const slot = slots[i];
    const bytesForSlot = bytesPerSlot + (i < remainder ? 1 : 0);
    slot.object[slot.key] += makeFiller(bytesForSlot, i);
  }

  const resizedJson = stringify(payload);
  const resizedBytes = byteLength(resizedJson);

  if (resizedBytes !== targetBytes) {
    throw new Error(`Could not produce exact size. Wanted ${targetBytes} bytes, got ${resizedBytes} bytes.`);
  }

  return resizedJson;
}

function printHelp() {
  const command = basename(process.argv[1]);

  console.log(`Usage: node scripts/${command} [--size 3mb] [--out ct-payload.json]

Options:
  -s, --size  Size in bytes, kb, kib, mb, or mib. Default: ${DEFAULT_SIZE}
  -o, --out   Output JSON file. Default: ${DEFAULT_OUT}

Examples:
  node scripts/${command}
  node scripts/${command} --size 3mib --out ct-payload.json
  node scripts/${command} --size 3145728 --out ct-payload.json`);
}

try {
  const args = parseArgs(process.argv.slice(2));
  const targetBytes = parseSize(args.size);
  const payload = makePayload();
  const json = resizePayload(payload, targetBytes);

  writeFileSync(args.out, json);
  console.log(`Wrote ${args.out} (${byteLength(json)} bytes)`);
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
}
