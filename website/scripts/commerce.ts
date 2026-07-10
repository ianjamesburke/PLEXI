/**
 * Operator surface for first-party monetization (stint 0355). A lightweight
 * internal seed/admin CLI — NOT the third-party submission queue (that is stint
 * 0344). It marks a first-party app paid at a given price (creating or updating
 * its Polar product) and ensures the Plexi AI Pro subscription product exists.
 *
 * Runs server-side where the Polar token and database live. Never ship the token
 * in the client binary. Invoke with vite-node so the TypeScript server modules
 * resolve without a build step:
 *
 *   POLAR_ACCESS_TOKEN=... POLAR_SERVER=sandbox DATABASE_URL=... \
 *     npm run commerce -- set-app <app_id> --price 4.99 --name "Reviewed Notes"
 *   POLAR_ACCESS_TOKEN=... POLAR_SERVER=sandbox \
 *     npm run commerce -- ensure-ai-pro
 *
 * Prices accept dollars ("4.99") or explicit cents ("--price 499 --cents").
 */
import { ensureAppProduct, ensureAiProProduct, AI_PRO_PRICE_CENTS } from '../src/server/products';
import { closePool } from '../src/server/db';

interface Flags {
  positional: string[];
  options: Record<string, string>;
  bools: Set<string>;
}

/** Parse `set-app foo --price 4.99 --name "X" --cents` into positionals/options/bools. */
function parseArgs(argv: string[]): Flags {
  const positional: string[] = [];
  const options: Record<string, string> = {};
  const bools = new Set<string>();
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next === undefined || next.startsWith('--')) {
        bools.add(key);
      } else {
        options[key] = next;
        i++;
      }
    } else {
      positional.push(arg);
    }
  }
  return { positional, options, bools };
}

/** Convert a price string to whole cents. Dollars by default; cents with --cents. */
function toCents(price: string, asCents: boolean): number {
  if (asCents) {
    const cents = Number.parseInt(price, 10);
    if (!Number.isInteger(cents)) throw new Error(`--price with --cents must be an integer, got "${price}"`);
    return cents;
  }
  const dollars = Number.parseFloat(price);
  if (!Number.isFinite(dollars)) throw new Error(`--price must be a number, got "${price}"`);
  return Math.round(dollars * 100);
}

function requireOption(flags: Flags, key: string): string {
  const value = flags.options[key];
  if (value === undefined) throw new Error(`--${key} is required`);
  return value;
}

const USAGE = `Usage:
  commerce set-app <app_id> --price <dollars> [--cents] [--currency usd] [--publisher plexi] [--name "Display Name"]
  commerce ensure-ai-pro [--price <dollars>] [--cents] [--currency usd]`;

async function main(): Promise<void> {
  const [command, ...rest] = process.argv.slice(2);
  const flags = parseArgs(rest);

  switch (command) {
    case 'set-app': {
      const appId = flags.positional[0];
      if (!appId) throw new Error('set-app requires an <app_id>');
      const priceCents = toCents(requireOption(flags, 'price'), flags.bools.has('cents'));
      const currency = flags.options.currency ?? 'usd';
      const publisher = flags.options.publisher ?? 'plexi';
      const name = flags.options.name ?? appId;
      const result = await ensureAppProduct({ appId, name, priceCents, currency, publisher });
      console.info(
        `[commerce-cli] app "${appId}" is now PAID at ${(priceCents / 100).toFixed(2)} ${currency.toUpperCase()} — ` +
          `product ${result.created ? 'created' : 'updated'} product_id=${result.productId}`,
      );
      break;
    }
    case 'ensure-ai-pro': {
      const priceCents = flags.options.price
        ? toCents(flags.options.price, flags.bools.has('cents'))
        : AI_PRO_PRICE_CENTS;
      const currency = flags.options.currency ?? 'usd';
      const result = await ensureAiProProduct({ priceCents, currency });
      console.info(
        `[commerce-cli] Plexi AI Pro product ${result.created ? 'created' : 'present'} product_id=${result.productId}. ` +
          `Wire it to /api/commerce/subscribe by setting POLAR_AI_PRO_PRODUCT_ID=${result.productId}`,
      );
      break;
    }
    default:
      console.error(USAGE);
      process.exitCode = 2;
      return;
  }
}

main()
  .catch((err) => {
    console.error('[commerce-cli] failed:', err instanceof Error ? err.message : err);
    process.exitCode = 1;
  })
  .finally(async () => {
    // ensure-ai-pro never touches the pool, but closePool is a safe no-op then.
    await closePool().catch(() => {});
  });
