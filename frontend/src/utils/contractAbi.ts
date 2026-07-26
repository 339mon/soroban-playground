export interface ContractAbiFunctionInput {
  name: string;
  type: string;
  doc?: string;
  enumVariants?: string[];
  structFields?: Array<{ name: string; type: string }>;
}

export interface ContractAbiFunction {
  name: string;
  doc?: string;
  inputs?: ContractAbiFunctionInput[];
  outputs?: string[];
}

export type ContractAbiValue = string | number | boolean | unknown[] | Record<string, unknown> | null | undefined;

const STELLAR_ADDRESS_REGEX = /^(G[A-Z2-7]{55}|C[A-Z2-7]{55})$/;
const SOROBAN_SYMBOL_REGEX = /^[a-zA-Z0-9_]{1,32}$/;

const RUST_TYPE_ALIASES: Record<string, string> = {
  symbol: "symbol",
  env: "string",
  address: "address",
  bytes: "string",
  string: "string",
  bool: "bool",
  u8: "number",
  u16: "number",
  u32: "number",
  u64: "u128",
  u128: "u128",
  u256: "u128",
  i8: "number",
  i16: "number",
  i32: "number",
  i64: "u128",
  i128: "u128",
  i256: "u128",
  f32: "number",
  f64: "number",
};

export function normalizeType(type: string): string {
  const normalized = type.trim().toLowerCase();

  if (RUST_TYPE_ALIASES[normalized]) {
    return RUST_TYPE_ALIASES[normalized];
  }

  if (normalized.startsWith("vec<") || normalized.startsWith("vector<")) return "vec";
  if (normalized.startsWith("map<")) return "map";
  if (normalized.startsWith("enum")) return "enum";
  if (normalized.startsWith("struct")) return "struct";

  return "string";
}

export function buildDefaultInputValue(type: string): ContractAbiValue {
  const kind = normalizeType(type);
  if (kind === "bool") return false;
  if (kind === "number") return 0;
  if (kind === "vec") return [];
  if (kind === "map" || kind === "struct") return {};
  return "";
}

export function validateSorobanType(name: string, type: string, value: unknown): string | null {
  const kind = normalizeType(type);

  if (value === undefined || value === null || value === "") {
    return `Field '${name}' (${type}) is required.`;
  }

  if (kind === "address") {
    const str = String(value).trim();
    if (!STELLAR_ADDRESS_REGEX.test(str)) {
      return `Field '${name}' must be a valid Stellar Address (G... or C..., 56 chars).`;
    }
  }

  if (kind === "symbol") {
    const str = String(value).trim();
    if (!SOROBAN_SYMBOL_REGEX.test(str)) {
      return `Field '${name}' must be a valid Soroban Symbol (max 32 alphanumeric/underscore chars).`;
    }
  }

  if (kind === "u128") {
    const str = String(value).trim();
    if (!/^-?\d+$/.test(str)) {
      return `Field '${name}' must be a valid 128-bit integer string or number.`;
    }
  }

  if (kind === "number") {
    if (Number.isNaN(Number(value))) {
      return `Field '${name}' must be a valid numeric value.`;
    }
  }

  if (kind === "vec") {
    if (!Array.isArray(value) && typeof value === "string") {
      try {
        const parsed = JSON.parse(value);
        if (!Array.isArray(parsed)) return `Field '${name}' must be a valid JSON array for Vec.`;
      } catch {
        return `Field '${name}' must be a valid JSON array for Vec.`;
      }
    }
  }

  return null;
}

export function parseContractAbiFromSource(source: string): ContractAbiFunction[] {
  const functionMatches = source.matchAll(/pub\s+fn\s+([A-Za-z0-9_]+)\s*\(([^)]*)\)/g);
  const functions: ContractAbiFunction[] = [];

  for (const match of functionMatches) {
    const [, name, paramsBlock] = match;
    const inputs = paramsBlock
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => {
        const [rawName, rawType] = part.split(":").map((val) => val.trim());
        if (!rawName || !rawType || rawName.startsWith("_") || rawName === "env" || rawName === "&env") {
          return null;
        }

        return {
          name: rawName.replace(/^&/, "").replace(/\s+/g, ""),
          type: rawType.replace(/\s+/g, ""),
        };
      })
      .filter(Boolean) as ContractAbiFunctionInput[];

    if (!name) continue;
    functions.push({ name, inputs });
  }

  return functions;
}

export function validateAbiArguments(
  abiFunction: ContractAbiFunction | null | undefined,
  values: Record<string, unknown>,
): string {
  if (!abiFunction) return "";

  for (const input of abiFunction.inputs ?? []) {
    const error = validateSorobanType(input.name, input.type, values[input.name]);
    if (error) return error;
  }

  return "";
}

export function buildAbiArguments(
  abiFunction: ContractAbiFunction | null | undefined,
  values: Record<string, unknown>,
): Record<string, unknown> {
  if (!abiFunction) return {};

  return (abiFunction.inputs ?? []).reduce<Record<string, unknown>>((args, input) => {
    const rawValue = values[input.name];
    const kind = normalizeType(input.type);

    if (kind === "bool") {
      args[input.name] = Boolean(rawValue);
    } else if (kind === "number") {
      args[input.name] = rawValue === "" || rawValue === undefined || rawValue === null ? 0 : Number(rawValue);
    } else if (kind === "vec" && typeof rawValue === "string") {
      try {
        args[input.name] = JSON.parse(rawValue);
      } catch {
        args[input.name] = [];
      }
    } else {
      args[input.name] = rawValue === undefined || rawValue === null ? "" : rawValue;
    }

    return args;
  }, {});
}
