import {
  AccountInfo,
  AccountMeta,
  ConfirmOptions,
  PublicKey,
  Signer,
  Transaction,
  TransactionInstruction,
  TransactionSignature,
} from "@solana/web3.js";
import { Buffer } from "buffer";
import {
  Idl,
  IdlInstructionAccount,
  IdlInstructionAccountItem,
  IdlInstructionAccounts,
  IdlTypeDef,
} from "../../idl.js";
import Provider from "../../provider.js";
import {
  AccountsGeneric,
  AccountsResolver,
  CustomAccountResolver,
} from "../accounts-resolver.js";
import { Address, translateAddress } from "../common.js";
import { Accounts } from "../context.js";
import { AccountNamespace } from "./account.js";
import { InstructionFn } from "./instruction.js";
import { RpcFn } from "./rpc.js";
import { SimulateFn, SimulateResponse } from "./simulate.js";
import { TransactionFn } from "./transaction.js";
import {
  AllInstructions,
  InstructionAccountAddresses,
  MakeMethodsNamespace,
  MethodsFn,
} from "./types.js";
import { ViewFn } from "./views.js";
import {
  createRpc,
  type Rpc,
  getDefaultAddressTreeInfo,
  TreeType,
  VERSION,
  featureFlags,
  deriveAddressV2,
  MerkleContext,
  parseTokenData,
  TreeInfo,
  deriveCompressionConfigAddress,
  getRegisteredProgramPda,
  deriveTokenProgramConfig,
} from "@lightprotocol/stateless.js";
import {
  AccountInput,
  buildDecompressParams,
  CompressedTokenProgram,
  getAccountInterface,
  getAtaInterface,
  CTOKEN_RENT_SPONSOR,
  getAssociatedCTokenAddressAndBump,
} from "@lightprotocol/compressed-token";

featureFlags.version = VERSION.V2;

export type MethodsNamespace<
  IDL extends Idl = Idl,
  I extends AllInstructions<IDL> = AllInstructions<IDL>
> = MakeMethodsNamespace<IDL, I>;

/**
 * Light Protocol compression metadata extracted from IDL
 */
interface LightCompressibleMetadata {
  compressibleAccounts: Map<
    string,
    {
      accountType: "pda" | "token";
      isATA?: boolean;
      seeds: Array<{ type: "const" | "account"; value: string }>;
    }
  >;
}

/**
 * Extract Light Protocol compression metadata from IDL enums.
 *
 * Parses CTokenAccountVariant and CompressedAccountVariant enums to determine:
 * - Which accounts are compressible
 * - Whether they're PDAs or tokens
 * - What their seed dependencies are
 */
function extractLightMetadata(idl: Idl): LightCompressibleMetadata | null {
  const compressibleAccounts = new Map<
    string,
    {
      accountType: "pda" | "token";
      isATA?: boolean;
      seeds: Array<{ type: "const" | "account"; value: string }>;
    }
  >();

  if (!idl.types) return null;

  const findAccountDef = (accounts: readonly any[], name: string): any => {
    for (const acc of accounts) {
      if ("accounts" in acc) {
        const found = findAccountDef(acc.accounts, name);
        if (found) return found;
      } else if (acc.name === name) {
        return acc;
      }
    }
    return null;
  };

  const extractPdaSeeds = (
    accountName: string
  ): Array<{ type: "const" | "account"; value: string }> => {
    for (const ix of idl.instructions) {
      const accountDef = findAccountDef(ix.accounts, accountName);
      if (accountDef && accountDef.pda && accountDef.pda.seeds) {
        const seeds: Array<{ type: "const" | "account"; value: string }> = [];

        // Check for cross-program PDA (unsupported)
        if (accountDef.pda.program) {
          throw new Error(
            `[decompressIfNeeded] Account '${accountName}' uses cross-program PDA derivation (seeds::program), which is not supported for auto-resolution.\n\n` +
              `Solution: Provide all accounts explicitly: .decompressIfNeeded({ ${accountName}: <address>, ... })`
          );
        }

        for (const seed of accountDef.pda.seeds) {
          if (seed.kind === "const" && seed.value) {
            // Convert byte array to UTF-8 string
            const constValue = Buffer.from(seed.value).toString("utf8");
            seeds.push({ type: "const", value: constValue });
          } else if (seed.kind === "arg") {
            // Instruction arguments are runtime values, not resolvable accounts
            throw new Error(
              `[decompressIfNeeded] Account '${accountName}' depends on instruction argument '${seed.path}', which cannot be auto-resolved.\n\n` +
                `Instruction arguments are runtime values you must provide.\n\n` +
                `Solutions:\n` +
                `  1. Provide the resolved account explicitly: .decompressIfNeeded({ ${accountName}: <derived_address>, ... })\n` +
                `  2. If auto-decompression isn't needed for this instruction, remove '${accountName}' from decompressIfNeeded()\n` +
                `  3. Build the transaction manually without decompressIfNeeded()`
            );
          } else if (seed.kind === "account") {
            // For account seeds, check if it's a field reference (path contains ".")
            // e.g., "pool_state.lp_mint" means: read lpMint field from poolState account
            if (seed.path && seed.path.includes(".")) {
              // Field reference: "account.field"
              // The field name (after the dot) is what we need as the seed account
              const parts = seed.path.split(".");
              if (parts.length >= 2) {
                // Convert to camelCase: "lp_mint" -> "lpMint"
                const fieldName = parts[parts.length - 1].replace(
                  /_([a-z0-9])/g,
                  (_, letter) => letter.toUpperCase()
                );
                seeds.push({ type: "account", value: fieldName });
              }
            } else {
              // Direct account reference (no field)
              const accountRef = seed.account || seed.path;
              if (accountRef) {
                const baseName = accountRef.split(".")[0].split("(")[0];
                seeds.push({ type: "account", value: baseName });
              }
            }
          }
        }
        return seeds;
      }
    }
    return [];
  };

  // c-token accounts
  const ctokenVariant = idl.types.find(
    (t) => t.name === "cTokenAccountVariant"
  );
  if (ctokenVariant && "variants" in ctokenVariant.type) {
    for (const variant of ctokenVariant.type.variants) {
      const variantName = variant.name; // Keep camelCase from IDL

      const seeds = extractPdaSeeds(variantName);
      const isATA = seeds.length === 0; // ATAs have no PDA seeds

      compressibleAccounts.set(variantName, {
        accountType: "token",
        isATA,
        seeds,
      });
    }
  }

  // compressed PDA accounts
  // Dedupe "packed*" variants (e.g., packedPoolState is same as poolState)
  const compressedVariant = idl.types.find(
    (t) => t.name === "compressedAccountVariant"
  );
  if (compressedVariant && "variants" in compressedVariant.type) {
    const seenBaseNames = new Set<string>();

    for (const variant of compressedVariant.type.variants) {
      const variantName = variant.name; // Keep camelCase from IDL

      // Skip wrapper types that contain token variants
      if (
        variantName.toLowerCase().includes("ctokendata") ||
        variantName.toLowerCase().includes("tokendata")
      ) {
        continue;
      }

      // Dedupe packed variants (e.g., packedPoolState -> poolState)
      let baseName = variantName;
      const isPacked = variantName.startsWith("packed");
      if (isPacked) {
        // Remove 'packed' prefix and lowercase first letter: packedPoolState -> poolState
        baseName = variantName.replace(/^packed/, "");
        baseName = baseName.charAt(0).toLowerCase() + baseName.slice(1);
        if (seenBaseNames.has(baseName)) {
          continue;
        }
      }

      let seeds = extractPdaSeeds(variantName);
      if (seeds.length === 0 && isPacked) {
        // Try unpacked version for seeds
        const unpackedVariantName = variantName.replace(/^packed/, "");
        seeds = extractPdaSeeds(unpackedVariantName);
      }

      seenBaseNames.add(baseName);
      compressibleAccounts.set(baseName, {
        accountType: "pda",
        isATA: false,
        seeds,
      });
    }
  }

  if (compressibleAccounts.size === 0) return null;

  console.log(
    `[Light] Extracted metadata for ${compressibleAccounts.size} compressible account(s):`
  );
  compressibleAccounts.forEach((meta, name) => {
    const seedsStr = meta.seeds
      .map((s) => (s.type === "const" ? `"${s.value}"` : s.value))
      .join(", ");
    console.log(`  - ${name}: type=${meta.accountType}, seeds=[${seedsStr}]`);
  });

  return { compressibleAccounts };
}

export class MethodsBuilderFactory {
  public static build<IDL extends Idl, I extends AllInstructions<IDL>>(
    provider: Provider,
    programId: PublicKey,
    idlIx: AllInstructions<IDL>,
    ixFn: InstructionFn<IDL>,
    txFn: TransactionFn<IDL>,
    rpcFn: RpcFn<IDL>,
    simulateFn: SimulateFn<IDL>,
    viewFn: ViewFn<IDL> | undefined,
    accountNamespace: AccountNamespace<IDL>,
    idlTypes: IdlTypeDef[],
    customResolver?: CustomAccountResolver<IDL>,
    idl?: IDL,
    allInstructionFns?: Record<string, InstructionFn<IDL>>
  ): MethodsFn<IDL, I, MethodsBuilder<IDL, I>> {
    return (...args) =>
      new MethodsBuilder(
        args,
        ixFn,
        txFn,
        rpcFn,
        simulateFn,
        viewFn,
        provider,
        programId,
        idlIx,
        accountNamespace,
        idlTypes,
        customResolver,
        idl,
        allInstructionFns
      );
  }
}

type ResolvedAccounts<
  A extends IdlInstructionAccountItem = IdlInstructionAccountItem
> = PartialUndefined<ResolvedAccountsRecursive<A>>;

type ResolvedAccountsRecursive<
  A extends IdlInstructionAccountItem = IdlInstructionAccountItem
> = OmitNever<{
  [N in A["name"]]: ResolvedAccount<A & { name: N }>;
}>;

type ResolvedAccount<
  A extends IdlInstructionAccountItem = IdlInstructionAccountItem
> = A extends IdlInstructionAccounts
  ? ResolvedAccountsRecursive<A["accounts"][number]>
  : A extends NonNullable<Pick<IdlInstructionAccount, "address">>
  ? never
  : A extends NonNullable<Pick<IdlInstructionAccount, "pda">>
  ? never
  : A extends NonNullable<Pick<IdlInstructionAccount, "relations">>
  ? never
  : A extends { signer: true }
  ? Address | undefined
  : PartialAccount<A>;

type PartialUndefined<
  T,
  P extends keyof T = {
    [K in keyof T]: undefined extends T[K] ? K : never;
  }[keyof T]
> = Partial<Pick<T, P>> & Pick<T, Exclude<keyof T, P>>;

type OmitNever<T extends Record<string, any>> = {
  [K in keyof T as T[K] extends never ? never : K]: T[K];
};

export type PartialAccounts<
  A extends IdlInstructionAccountItem = IdlInstructionAccountItem
> = Partial<{
  [N in A["name"]]: PartialAccount<A & { name: N }>;
}>;

type PartialAccount<
  A extends IdlInstructionAccountItem = IdlInstructionAccountItem
> = A extends IdlInstructionAccounts
  ? PartialAccounts<A["accounts"][number]>
  : A extends { optional: true }
  ? Address | null
  : Address;

export function isPartialAccounts(
  partialAccount: any
): partialAccount is PartialAccounts {
  return (
    typeof partialAccount === "object" &&
    partialAccount !== null &&
    !("_bn" in partialAccount) // Ensures not a pubkey
  );
}

export function flattenPartialAccounts<A extends IdlInstructionAccountItem>(
  partialAccounts: PartialAccounts<A>,
  throwOnNull: boolean
): AccountsGeneric {
  const toReturn: AccountsGeneric = {};
  for (const accountName in partialAccounts) {
    const account = partialAccounts[accountName];
    if (account === null) {
      if (throwOnNull)
        throw new Error(
          "Failed to resolve optionals due to IDL type mismatch with input accounts!"
        );
      continue;
    }
    toReturn[accountName] = isPartialAccounts(account)
      ? flattenPartialAccounts(account, true)
      : translateAddress(account);
  }
  return toReturn;
}

// Helper to extract decompress instruction accounts type
type DecompressAccounts<IDL extends Idl> = Extract<
  AllInstructions<IDL>,
  { name: "decompressAccountsIdempotent" }
> extends { accounts: readonly any[] }
  ? Extract<
      AllInstructions<IDL>,
      { name: "decompressAccountsIdempotent" }
    >["accounts"][number]
  : never;

// Helper to extract the decompress instruction type
type DecompressInstruction<IDL extends Idl> = Extract<
  AllInstructions<IDL>,
  { name: "decompressAccountsIdempotent" }
>;

export class MethodsBuilder<
  IDL extends Idl,
  I extends AllInstructions<IDL>,
  A extends I["accounts"][number] = I["accounts"][number],
  D extends DecompressAccounts<IDL> = DecompressAccounts<IDL>
> {
  private _accounts: AccountsGeneric = {};
  private _remainingAccounts: Array<AccountMeta> = [];
  private _signers: Array<Signer> = [];
  private _preInstructions: Array<TransactionInstruction> = [];
  private _postInstructions: Array<TransactionInstruction> = [];
  private _accountsResolver: AccountsResolver<IDL>;
  private _resolveAccounts: boolean = true;
  private _enableAutoDecompress: boolean = false;
  private _decompressAccounts?: AccountsGeneric;
  private _idl?: IDL;
  private _idlIx: AllInstructions<IDL>;
  private _idlTypes: IdlTypeDef[];
  private _allInstructionFns?: Record<string, InstructionFn<IDL>>;
  private _programId: PublicKey;
  private _accountNamespace: AccountNamespace<IDL>;
  private _lightMetadata?: LightCompressibleMetadata | null;
  private _ataDerivations: Record<
    string,
    { owner: PublicKey; mint: PublicKey }
  > = {};

  constructor(
    private _args: Array<any>,
    private _ixFn: InstructionFn<IDL>,
    private _txFn: TransactionFn<IDL>,
    private _rpcFn: RpcFn<IDL>,
    private _simulateFn: SimulateFn<IDL>,
    private _viewFn: ViewFn<IDL> | undefined,
    provider: Provider,
    programId: PublicKey,
    idlIx: AllInstructions<IDL>,
    accountNamespace: AccountNamespace<IDL>,
    idlTypes: IdlTypeDef[],
    customResolver?: CustomAccountResolver<IDL>,
    idl?: IDL,
    allInstructionFns?: Record<string, InstructionFn<IDL>>
  ) {
    this._programId = programId;
    this._idl = idl;
    this._idlIx = idlIx;
    this._idlTypes = idlTypes;
    this._allInstructionFns = allInstructionFns;
    this._accountNamespace = accountNamespace;
    this._accountsResolver = new AccountsResolver(
      _args,
      this._accounts,
      provider,
      programId,
      idlIx,
      accountNamespace,
      idlTypes,
      customResolver
    );

    // Extract Light Protocol compression metadata from IDL if available
    if (idl) {
      console.log("Extracting Light Protocol compression metadata from IDL");
      this._lightMetadata = extractLightMetadata(idl);
      console.log(
        "Light Protocol compression metadata extracted..",
        this._lightMetadata
      );
    }
  }

  public args(args: Array<any>): void {
    this._args = args;
    this._accountsResolver.args(args);
  }

  /**
   * Set instruction accounts with account resolution.
   *
   * This method only accepts accounts that cannot be resolved.
   *
   * See {@link accountsPartial} for overriding the account resolution or
   * {@link accountsStrict} for strictly specifying all accounts.
   */
  public accounts(accounts: ResolvedAccounts<A>) {
    // @ts-ignore
    return this.accountsPartial(accounts);
  }

  /**
   * Set instruction accounts with account resolution.
   *
   * There is no functional difference between this method and {@link accounts}
   * method, the only difference is this method allows specifying all accounts
   * even if they can be resolved. On the other hand, {@link accounts} method
   * doesn't accept accounts that can be resolved.
   */
  public accountsPartial(accounts: PartialAccounts<A>) {
    this._resolveAccounts = true;
    this._accountsResolver.resolveOptionals(accounts);
    return this;
  }

  /**
   * Set instruction accounts without account resolution.
   *
   * All accounts strictly need to be specified when this method is used.
   *
   * See {@link accounts} and {@link accountsPartial} methods for automatically
   * resolving accounts.
   *
   * @param accounts instruction accounts
   */
  public accountsStrict(accounts: Accounts<A>) {
    this._resolveAccounts = false;
    this._accountsResolver.resolveOptionals(accounts);
    return this;
  }

  /**
   * Set instruction signers.
   *
   * Note that calling this method appends the given signers to the existing
   * signers (instead of overriding them).
   *
   * @param signers signers to append
   */
  public signers(signers: Array<Signer>) {
    this._signers = this._signers.concat(signers);
    return this;
  }

  /**
   * Set remaining accounts.
   *
   * Note that calling this method appends the given accounts to the existing
   * remaining accounts (instead of overriding them).
   *
   * @param accounts remaining accounts
   */
  public remainingAccounts(accounts: Array<AccountMeta>) {
    this._remainingAccounts = this._remainingAccounts.concat(accounts);
    return this;
  }

  /**
   * Set previous instructions.
   *
   * See {@link postInstructions} to set the post instructions instead.
   *
   * @param ixs instructions
   * @param prepend whether to prepend to the existing previous instructions
   */
  public preInstructions(ixs: Array<TransactionInstruction>, prepend = false) {
    if (prepend) {
      this._preInstructions = ixs.concat(this._preInstructions);
    } else {
      this._preInstructions = this._preInstructions.concat(ixs);
    }
    return this;
  }

  /**
   * Set post instructions.
   *
   * See {@link preInstructions} to set the previous instructions instead.
   *
   * @param ixs instructions
   */
  public postInstructions(ixs: Array<TransactionInstruction>) {
    this._postInstructions = this._postInstructions.concat(ixs);
    return this;
  }

  /**
   * Enable automatic decompression for compressible accounts via IDL.
   *
   * Resolution priority for each account:
   * 1. Explicitly provided in decompressIfNeeded() call
   * 2. Auto-pulled from main instruction (same name)
   * 3. Auto-resolved constants (ctokenProgram, ctokenCpiAuthority, config,
   *    etc.)
   * 4. Error if required but not found
   *
   * @param accounts Partial accounts for decompressAccountsIdempotent
   * @returns MethodsBuilder<IDL, I, A, D> for chaining
   *
   * @example
   * ```ts
   * // Auto-resolve everything (feePayer from provider.wallet)
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded()
   *   .accounts({ ... })
   *   .rpc();
   *
   * // Minimal - provide only what can't be auto-resolved
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded({
   *     feePayer: owner.publicKey,
   *     rentPayer: owner.publicKey,
   *   })
   *   .accounts({ ... })
   *   .rpc();
   *
   * // Explicit - override specific accounts
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded({
   *     feePayer: owner.publicKey,
   *     rentPayer: owner.publicKey,
   *     ammConfig: configAddress,
   *     token0Mint: inputToken,
   *     token1Mint: outputToken,
   *   })
   *   .accounts({ ... })
   *   .rpc();
   * ```
   */
  public decompressIfNeeded(
    accounts: Partial<Accounts<D>> & { [name: string]: Address } = {}
  ) {
    this._enableAutoDecompress = true;
    this._decompressAccounts = flattenPartialAccounts(accounts as any, false);
    return this;
  }

  /**
   * Get the public keys of the instruction accounts.
   *
   * The return type is an object with account names as keys and their public
   * keys as their values.
   *
   * Note that an account key is `undefined` if the account hasn't yet been
   * specified or resolved.
   */
  public async pubkeys(): Promise<
    Partial<InstructionAccountAddresses<IDL, I>>
  > {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }
    // @ts-ignore
    return this._accounts;
  }

  /**
   * Fetch and check compression state for provided accounts.
   * Returns AccountInput[] for buildDecompressParams.
   *
   * For ATA variants (seedless tokens), derives owner+mint from main instruction
   * and queries by owner+mint instead of address.
   */
  private async _fetchCompressibleAccounts(
    accountsToCheck: Array<{
      name: string;
      address: PublicKey;
      ataDerivation?: { owner: PublicKey; mint: PublicKey };
    }>
  ): Promise<AccountInput[]> {
    if (accountsToCheck.length === 0) return [];

    const programId = this._programId;
    const rpc = createRpc();

    const defaultAddressTreeInfo = getDefaultAddressTreeInfo();

    console.log(
      `[decompressIfNeeded] Fetching compression state for ${accountsToCheck.length} account(s)...`
    );

    // Prepare an array of concurrent fetch promises
    const fetchPromises = accountsToCheck.map(
      async ({ name, address, ataDerivation }) => {
        console.log(
          `[decompressIfNeeded]   → Checking '${name}' (${address.toBase58()})...`
        );

        // Check if this is an ATA variant (seedless token)
        const metadata = this._lightMetadata?.compressibleAccounts.get(name);
        if (metadata?.isATA) {
          console.log(
            `[decompressIfNeeded]     → ATA variant detected, querying by owner+mint`
          );

          // Prefer derived owner+mint passed from resolution stage
          const owner = ataDerivation?.owner || null;
          const mint = ataDerivation?.mint || null;

          if (!owner || !mint) {
            console.log(
              `[decompressIfNeeded]     ✗ ATA requires derived owner+mint (ambiguous or not resolved)`
            );
            return undefined;
          }

          try {
            const ataResult = await getAtaInterface(
              rpc,
              owner,
              mint,
              undefined
            );

            if (ataResult.isCompressed && ataResult.merkleContext) {
              console.log(`[decompressIfNeeded]     ✓ ATA is COMPRESSED`);
              return {
                address: ataResult.parsed.address,
                info: {
                  accountInfo: ataResult.accountInfo,
                  parsed: ataResult.parsed,
                  merkleContext: ataResult.merkleContext,
                },
                accountType: "cTokenData",
                tokenVariant: name,
              } as AccountInput;
            } else {
              console.log(`[decompressIfNeeded]     ✗ ATA is NOT compressed`);
            }
          } catch (err) {
            console.log(
              `[decompressIfNeeded]     ✗ ATA fetch failed: ${
                (err as Error).message
              }`
            );
          }

          return undefined;
        }

        // name is already camelCase from IDL metadata

        try {
          // Try fetching as CPDA first
          // @ts-ignore
          const cpdaResult = await rpc.getAccountInfoInterface(
            address,
            programId,
            //@ts-ignore
            defaultAddressTreeInfo
          );

          if (
            cpdaResult &&
            cpdaResult.merkleContext &&
            cpdaResult.isCompressed
          ) {
            console.log(
              `[decompressIfNeeded]     ✓ Account '${name}' is COMPRESSED`
            );
            // It's a compressed account
            if (
              cpdaResult.accountInfo.owner.equals(
                CompressedTokenProgram.programId
              )
            ) {
              // It's a compressed token account
              try {
                const parsed = parseTokenData(cpdaResult.accountInfo.data);
                if (parsed) {
                  return {
                    address,
                    info: {
                      accountInfo: cpdaResult.accountInfo,
                      parsed,
                      merkleContext: cpdaResult.merkleContext,
                    },
                    accountType: "cTokenData",
                    tokenVariant: name,
                  } as AccountInput;
                }
              } catch {
                // Not a token account, ignore
              }
            } else {
              // It's a compressed program account
              let parsed = null;

              // Use camelCase name for account client lookup
              const accountClient = (this._accountNamespace as any)[name];

              console.log(
                `[decompressIfNeeded]     → Trying to parse '${name}' using account client '${name}'`
              );

              if (accountClient?.coder) {
                try {
                  parsed = accountClient.coder.accounts.decode(
                    name,
                    cpdaResult.accountInfo.data
                  );
                  console.log(
                    `[decompressIfNeeded]     ✓ Parsed successfully using decode()`
                  );
                } catch {
                  try {
                    parsed = accountClient.coder.accounts.decodeAny(
                      cpdaResult.accountInfo.data
                    );
                    console.log(
                      `[decompressIfNeeded]     ✓ Parsed successfully using decodeAny()`
                    );
                  } catch (e) {
                    console.log(
                      `[decompressIfNeeded]     ✗ Parsing failed: ${
                        (e as Error).message
                      }`
                    );
                  }
                }
              } else {
                console.log(
                  `[decompressIfNeeded]     ✗ No account client found for '${name}'`
                );
              }

              return {
                address,
                info: {
                  accountInfo: cpdaResult.accountInfo,
                  parsed,
                  merkleContext: cpdaResult.merkleContext,
                },
                accountType: name,
              } as AccountInput;
            }
          } else {
            console.log(
              `[decompressIfNeeded]     ✗ Not compressed via getAccountInfoInterface, trying token fallback...`
            );
            // Try as compressed token via getAccountInterface as fallback
            try {
              const tokenRes = await getAccountInterface(
                rpc,
                address,
                undefined,
                CompressedTokenProgram.programId
              );
              if (tokenRes && (tokenRes as any).merkleContext) {
                console.log(
                  `[decompressIfNeeded]     ✓ Found via token fallback - COMPRESSED token`
                );
                return {
                  address,
                  info: tokenRes as any,
                  accountType: "cTokenData",
                  tokenVariant: name,
                } as AccountInput;
              } else {
                console.log(
                  `[decompressIfNeeded]     ✗ Account '${name}' is NOT compressed (no merkleContext)`
                );
              }
            } catch (err) {
              console.log(
                `[decompressIfNeeded]     ✗ Token fallback failed: ${
                  (err as Error).message
                }`
              );
            }
          }
        } catch (err) {
          console.log(
            `[decompressIfNeeded]     ✗ Error fetching '${name}': ${
              (err as Error).message
            }`
          );
        }
        // Return undefined if no compressed account found
        return undefined;
      }
    );

    // Run all fetchers concurrently
    const results = await Promise.allSettled(fetchPromises);

    // Collect all successful AccountInput results (move .value up to array)
    const accountInputs: AccountInput[] = [];
    results.forEach((result, idx) => {
      if (result.status === "fulfilled" && result.value) {
        accountInputs.push(result.value as AccountInput);
      }
      // (rejected results already logged above inside each promise's catch)
    });

    console.log(
      `[decompressIfNeeded] Found ${accountInputs.length} compressed account(s):`
    );
    accountInputs.forEach((acc, idx) => {
      console.log(
        `  [${idx}] ${acc.address.toBase58()} - type: ${
          acc.accountType
        }, variant: ${acc.tokenVariant || "N/A"}`
      );
    });

    return accountInputs;
  }

  /**
   * Helper to get account from main instruction if it exists with the same name
   */
  private _getFromMainInstruction(accountName: string): PublicKey | null {
    const programId = this._programId;
    if (this._accounts && accountName in this._accounts) {
      try {
        const pubkey = translateAddress(this._accounts[accountName] as Address);
        if (!pubkey.equals(programId)) {
          return pubkey;
        }
      } catch {
        // Invalid address, ignore
      }
    }
    return null;
  }

  /**
   * Auto-resolve constant and default accounts for decompressAccountsIdempotent.
   * Priority: 1) User explicit values in decompressIfNeeded, 2) Main instruction accounts, 3) Auto-resolved defaults
   */
  private _autoResolveDecompressAccounts(): void {
    if (!this._decompressAccounts) return;

    const programId = this._programId;

    // Helper to check if an account is missing or set to program ID (placeholder)
    const isAccountMissing = (name: string): boolean => {
      if (!this._decompressAccounts || !(name in this._decompressAccounts))
        return true;
      try {
        const pubkey = translateAddress(
          this._decompressAccounts![name] as Address
        );
        return pubkey.equals(programId);
      } catch {
        return true;
      }
    };

    // Constants (ALWAYS these values - enforced by Rust constraints)
    const CTOKEN_PROGRAM_ID = new PublicKey(
      "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m"
    );
    const CTOKEN_CPI_AUTHORITY_PDA = new PublicKey(
      "GXtd2izAiMJPwMEjfgTRH3d7k9mjn4Jq3JrWFv9gySYy"
    );

    // Auto-resolve ctokenProgram (check main instruction first)
    if (isAccountMissing("ctokenProgram")) {
      const fromMain = this._getFromMainInstruction("ctokenProgram");
      if (fromMain && fromMain.equals(CTOKEN_PROGRAM_ID)) {
        this._decompressAccounts.ctokenProgram = fromMain;
      } else {
        console.log(
          "[decompressIfNeeded] Auto-resolving ctokenProgram to constant:",
          CTOKEN_PROGRAM_ID.toBase58()
        );
        this._decompressAccounts.ctokenProgram = CTOKEN_PROGRAM_ID;
      }
    }

    // Auto-resolve ctokenCpiAuthority (check main instruction first)
    if (isAccountMissing("ctokenCpiAuthority")) {
      const fromMain = this._getFromMainInstruction("ctokenCpiAuthority");
      if (fromMain && fromMain.equals(CTOKEN_CPI_AUTHORITY_PDA)) {
        this._decompressAccounts.ctokenCpiAuthority = fromMain;
      } else {
        console.log(
          "[decompressIfNeeded] Auto-resolving ctokenCpiAuthority to constant:",
          CTOKEN_CPI_AUTHORITY_PDA.toBase58()
        );
        this._decompressAccounts.ctokenCpiAuthority = CTOKEN_CPI_AUTHORITY_PDA;
      }
    }

    // Auto-resolve config (check main instruction first, then derive)
    console.log("CHECKING if CONFIG IS MISSING");
    if (isAccountMissing("config")) {
      console.log("CONFIG IS MISSING");
      const fromMain = this._getFromMainInstruction("config");
      if (fromMain) {
        console.log("CONFIG IS in MAIN");
        console.log("FROM MAIN:", fromMain.toString());
        this._decompressAccounts.config = fromMain;
      } else {
        // Derive compression config PDA owned by the user's program.
        // defaults to index 0
        const [configPda] = deriveCompressionConfigAddress(programId);
        console.log("programId:", programId.toString());
        console.log(
          "SHOULD BE THE SAME:",
          deriveCompressionConfigAddress(programId)[0].toString()
        );
        console.log(
          "[decompressIfNeeded] Auto-resolving config to program's config PDA:",
          configPda.toBase58(),
          "(Note: must be initialized first via initialize_compression_config)"
        );
        this._decompressAccounts.config = configPda;
      }
    }

    // Auto-resolve feePayer with strict precedence order
    if (isAccountMissing("feePayer")) {
      let feePayer: PublicKey | null = null;

      // Priority 1: Check main instruction (in case user explicitly passed it there)
      const fromMain = this._getFromMainInstruction("feePayer");
      if (fromMain) {
        feePayer = fromMain;
        console.log(
          "[decompressIfNeeded] Using feePayer from main instruction:",
          feePayer.toBase58()
        );
      }
      // Priority 2: Try payer
      const fromPayer = this._getFromMainInstruction("payer");
      if (fromPayer) {
        feePayer = fromPayer;
        console.log(
          "[decompressIfNeeded] Using payer as feePayer from main instruction:",
          feePayer.toBase58()
        );
      }
      // Priority 3: Use provider wallet
      else {
        try {
          const provider = (this._accountsResolver as any)["_provider"];
          const walletPubkey = provider?.wallet?.publicKey;
          if (walletPubkey) {
            feePayer = walletPubkey;
            console.log(
              "[decompressIfNeeded] Using provider wallet as feePayer (transaction signer):",
              walletPubkey.toBase58()
            );
          }
        } catch {
          // Provider not available or doesn't have wallet
        }
      }

      // Priority 3: Error - cannot proceed without feePayer
      // Note: We do NOT fallback to "first instruction signer" because:
      //   - Transaction feePayer ≠ Instruction signers
      //   - First signer might be multisig authority, not the actual fee payer
      //   - This would cause subtle bugs and is not robust
      if (!feePayer) {
        throw new Error(
          `[decompressIfNeeded] Unable to determine feePayer for decompression.\n\n` +
            `The feePayer pays transaction fees and must match the transaction signer.\n\n` +
            `Solutions:\n` +
            `  1. Provide explicitly: .decompressIfNeeded({ feePayer: <publicKey>, ... })\n` +
            `  2. Ensure your provider has a wallet set (standard Anchor requirement)\n\n` +
            `Note: The feePayer must be the transaction signer (provider.wallet), not just any instruction signer.`
        );
      }

      this._decompressAccounts.feePayer = feePayer;
    }

    // Auto-resolve rentPayer (defaults to feePayer if not provided)
    if (isAccountMissing("rentPayer")) {
      // First check if feePayer was resolved
      const feePayer = this._decompressAccounts.feePayer
        ? translateAddress(this._decompressAccounts.feePayer as Address)
        : null;

      if (feePayer) {
        console.log(
          "[decompressIfNeeded] Defaulting rentPayer to feePayer:",
          feePayer.toBase58()
        );
        this._decompressAccounts.rentPayer = feePayer;
      } else {
        throw new Error(
          `[decompressIfNeeded] Unable to determine rentPayer (feePayer must be resolved first)`
        );
      }
    }

    // Auto-resolve ctokenRentSponsor (check main instruction first, then fallback to feePayer)
    if (isAccountMissing("ctokenRentSponsor")) {
      const fromMain = this._getFromMainInstruction("ctokenRentSponsor");
      if (fromMain) {
        this._decompressAccounts.ctokenRentSponsor = fromMain;
      } else {
        console.log(
          "[decompressIfNeeded] Auto-resolving ctokenRentSponsor to cosntant CTOKEN_RENT_SPONSOR:",
          CTOKEN_RENT_SPONSOR.toBase58()
        );
        this._decompressAccounts.ctokenRentSponsor = CTOKEN_RENT_SPONSOR;
      }
    }

    // Auto-resolve ctokenConfig (check main instruction first, then derive)
    if (isAccountMissing("ctokenConfig")) {
      const fromMain = this._getFromMainInstruction("ctokenConfig");
      if (fromMain) {
        this._decompressAccounts.ctokenConfig = fromMain;
      } else {
        // Derive ctoken config PDA using the correct SDK function
        const [ctokenConfigPda] = deriveTokenProgramConfig();
        console.log(
          "[decompressIfNeeded] Auto-resolving ctokenConfig to default PDA:",
          ctokenConfigPda.toBase58()
        );
        this._decompressAccounts.ctokenConfig = ctokenConfigPda;
      }
    }
  }

  /**
   * Flatten IDL account metadata for the current instruction, preserving signer flags.
   */
  private _collectIdlAccountMetas(): Array<{ name: string; signer: boolean }> {
    const metas: Array<{ name: string; signer: boolean }> = [];
    const ix: any = this._idlIx as any;
    const walk = (items: any[]) => {
      for (const item of items || []) {
        if (typeof item === "string") continue;
        if ("accounts" in item && Array.isArray(item.accounts)) {
          walk(item.accounts);
        } else if (item && typeof item === "object" && item.name) {
          metas.push({ name: item.name, signer: !!item.signer });
        }
      }
    };
    if (ix && Array.isArray(ix.accounts)) {
      walk(ix.accounts);
    }
    return metas;
  }

  /**
   * Brute-force resolve an ATA by trying all owner/mint permutations from main accounts.
   * Returns the first unambiguous match whose derived cATA exists in the main accounts.
   */
  private _resolveAtaByBruteForce(accountName: string): {
    found: boolean;
    address?: PublicKey;
    owner?: PublicKey;
    mint?: PublicKey;
  } {
    const programId = this._programId;
    const idlMetas = this._collectIdlAccountMetas();

    // Collect candidate owners: signer accounts + heuristic names
    const ownerNameHints = new Set([
      "owner",
      "payer",
      "user",
      "authority",
      "admin",
    ]);

    const ownerCandidates: PublicKey[] = [];
    const seenOwners = new Set<string>();
    for (const meta of idlMetas) {
      const nameLower = String(meta.name || "").toLowerCase();
      const hinted = [...ownerNameHints].some((h) => nameLower.includes(h));
      if (!meta.signer && !hinted) continue;
      if (!this._accounts || !(meta.name in this._accounts)) continue;
      try {
        const pk = translateAddress(this._accounts[meta.name] as Address);
        if (!pk.equals(programId)) {
          const s = pk.toBase58();
          if (!seenOwners.has(s)) {
            ownerCandidates.push(pk);
            seenOwners.add(s);
          }
        }
      } catch {}
    }

    // Collect candidate mints: account names containing "mint"
    const mintCandidates: PublicKey[] = [];
    const seenMints = new Set<string>();
    if (this._accounts) {
      for (const [name, value] of Object.entries(this._accounts)) {
        const nameLower = name.toLowerCase();
        if (!nameLower.includes("mint")) continue;
        try {
          const pk = translateAddress(value as Address);
          if (!pk.equals(programId)) {
            const s = pk.toBase58();
            if (!seenMints.has(s)) {
              mintCandidates.push(pk);
              seenMints.add(s);
            }
          }
        } catch {}
      }
    }

    if (ownerCandidates.length === 0 || mintCandidates.length === 0) {
      return { found: false };
    }

    // Build a set of all main accounts for matching
    const mainAddresses = new Set<string>();
    if (this._accounts) {
      for (const value of Object.values(this._accounts)) {
        try {
          const pk = translateAddress(value as Address);
          if (!pk.equals(programId)) mainAddresses.add(pk.toBase58());
        } catch {}
      }
    }

    let match: {
      address: PublicKey;
      owner: PublicKey;
      mint: PublicKey;
    } | null = null;

    for (const owner of ownerCandidates) {
      for (const mint of mintCandidates) {
        try {
          const [derivedAta] = getAssociatedCTokenAddressAndBump(owner, mint);
          const derivedStr = derivedAta.toBase58();
          if (mainAddresses.has(derivedStr)) {
            // If we've already found a different match, it's ambiguous → bail
            if (match && !match.address.equals(derivedAta)) {
              return { found: false };
            }
            match = { address: derivedAta, owner, mint };
          }
        } catch {}
      }
    }

    if (!match) return { found: false };
    // Store for fetch stage
    this._ataDerivations[accountName] = {
      owner: match.owner,
      mint: match.mint,
    };
    return {
      found: true,
      address: match.address,
      owner: match.owner,
      mint: match.mint,
    };
  }

  /**
   * Internal method to inject decompress instruction if needed.
   *
   * Uses _lightMetadata (extracted from IDL enums) as canonical reference for all compressible accounts.
   * Auto-resolves accounts by matching seed dependencies against main instruction accounts.
   */
  private async _injectDecompressIfNeeded(): Promise<void> {
    if (!this._enableAutoDecompress || !this._decompressAccounts) return;

    if (!this._idl || !this._allInstructionFns) {
      console.warn(
        "[decompressIfNeeded] Missing IDL or instruction functions."
      );
      return;
    }

    if (
      !this._lightMetadata ||
      this._lightMetadata.compressibleAccounts.size === 0
    ) {
      console.log(
        "[decompressIfNeeded] No compressible accounts metadata found in IDL. Skipping decompress."
      );
      return;
    }

    const decompressInstruction = this._idl.instructions.find(
      (ix: any) => ix.name === "decompressAccountsIdempotent"
    );

    if (!decompressInstruction) {
      console.warn(
        "[decompressIfNeeded] decompressAccountsIdempotent instruction not found in IDL."
      );
      return;
    }

    console.log("[decompressIfNeeded] Starting auto-decompression...");
    console.log(
      `[decompressIfNeeded] Using Light metadata with ${this._lightMetadata.compressibleAccounts.size} compressible account(s) as canonical reference`
    );

    const programId = this._programId;

    // System accounts that are infrastructure (not compressible)
    const SYSTEM_ACCOUNTS = new Set([
      "feePayer",
      "config",
      "rentPayer",
      "ctokenRentSponsor",
      "ctokenProgram",
      "ctokenCpiAuthority",
      "ctokenConfig",
      "compressionAuthority",
      "ctokenCompressionAuthority",
    ]);

    // Step 1: Auto-resolve constants and defaults for system accounts
    this._autoResolveDecompressAccounts();

    // Step 2: Use Light metadata to resolve ALL compressible accounts
    const completeAccounts: Record<string, PublicKey> = {};
    const compressibleAccountsToCheck: Array<{
      name: string;
      address: PublicKey;
      ataDerivation?: { owner: PublicKey; mint: PublicKey };
    }> = [];
    const resolvedAddresses = new Set<string>(); // Track which addresses have been claimed

    // First, resolve all compressible accounts from Light metadata
    console.log(
      `[decompressIfNeeded] Main instruction accounts available: [${
        this._accounts ? Object.keys(this._accounts).join(", ") : "none"
      }]`
    );

    for (const [
      accountName,
      metadata,
    ] of this._lightMetadata.compressibleAccounts.entries()) {
      console.log(
        `\n[decompressIfNeeded] ===== Resolving '${accountName}' (${metadata.accountType}) =====`
      );
      let resolved: PublicKey | null = null;

      // Priority 1: Check if explicitly provided in decompressIfNeeded()
      if (accountName in this._decompressAccounts) {
        try {
          const provided = translateAddress(
            this._decompressAccounts[accountName] as Address
          );
          if (!provided.equals(programId)) {
            resolved = provided;
            console.log(
              `[decompressIfNeeded] ✓ P1 Explicit: ${provided.toBase58()}`
            );
          }
        } catch {
          console.log(`[decompressIfNeeded] ✗ P1 Explicit: invalid address`);
        }
      } else {
        console.log(`[decompressIfNeeded] ✗ P1 Explicit: not provided`);
      }

      // Priority 2: Try name match from main instruction
      if (!resolved) {
        const fromMain = this._getFromMainInstruction(accountName);
        if (fromMain) {
          console.log(
            `[decompressIfNeeded] ✓ P2 Name match: found in main instruction as '${accountName}' → ${fromMain.toBase58()}`
          );
          resolved = fromMain;
        } else {
          console.log(
            `[decompressIfNeeded] ✗ P2 Name match: '${accountName}' not in main instruction`
          );
        }
      }

      // Priority 3: Use seed-based resolution with Light metadata
      if (!resolved && metadata.seeds.length > 0) {
        const seedsStr = metadata.seeds
          .map((s) => (s.type === "const" ? `"${s.value}"` : s.value))
          .join(", ");
        console.log(
          `[decompressIfNeeded] → P3 Seed-based: trying seeds [${seedsStr}]`
        );

        const resolveResult = this._resolveCompressibleAccountFromMetadata(
          accountName,
          metadata
        );

        if (resolveResult.found) {
          resolved = resolveResult.address!;
          console.log(
            `[decompressIfNeeded] ✓ P3 Seed-based: resolved → ${resolved.toBase58()}`
          );

          // Store resolved seed accounts in completeAccounts
          for (const [seedName, seedValue] of Object.entries(
            resolveResult.seeds || {}
          )) {
            if (!(seedName in completeAccounts)) {
              completeAccounts[seedName] = seedValue;
              console.log(
                `[decompressIfNeeded]   └─ seed '${seedName}' → ${seedValue.toBase58()}`
              );
            }
          }
        } else {
          console.log(
            `[decompressIfNeeded] ✗ P3 Seed-based: failed (seeds not found or derivation failed)`
          );
        }
      } else if (!resolved && metadata.seeds.length === 0) {
        console.log(
          `[decompressIfNeeded] ✗ P3 Seed-based: no seeds defined for this account`
        );
        // If this is a seedless token, attempt ATA brute-force resolution by owner/mint
        if (metadata.accountType === "token") {
          console.log(
            `[decompressIfNeeded] → Attempting ATA brute-force resolution for '${accountName}'`
          );
          const ata = this._resolveAtaByBruteForce(accountName);
          if (ata.found && ata.address) {
            resolved = ata.address;
            console.log(
              `[decompressIfNeeded] ✓ ATA brute-force: derived ${resolved.toBase58()} (owner=${ata.owner?.toBase58()}, mint=${ata.mint?.toBase58()})`
            );
          } else {
            console.log(
              `[decompressIfNeeded] ✗ ATA brute-force: no unambiguous match`
            );
          }
        }
      }

      // Priority 4: Try brute-force matching by trying all combinations of accounts as seeds
      if (!resolved && this._accounts) {
        const numMainAccounts = Object.keys(this._accounts).length;
        console.log(
          `[decompressIfNeeded] → P4 Brute-force: trying all combinations with ${numMainAccounts} main account(s)...`
        );

        const mainAccountEntries = Object.entries(this._accounts);

        if (metadata.accountType === "token" && metadata.seeds.length > 0) {
          // For token accounts with seeds, try to derive using seed combinations
          console.log(
            `[decompressIfNeeded]   ├─ Token account with ${metadata.seeds.length} seed(s): trying seed combinations...`
          );

          const accountSeeds = metadata.seeds.filter(
            (s) => s.type === "account"
          );
          const expectedSeedCount = accountSeeds.length;
          const seedsStr = metadata.seeds
            .map((s) => (s.type === "const" ? `"${s.value}"` : s.value))
            .join(", ");
          console.log(
            `[decompressIfNeeded]   │  Need ${expectedSeedCount} account seed(s): [${seedsStr}]`
          );

          // Try all combinations of accounts as potential seeds
          const tryDerivation = (
            seedIndices: number[]
          ): {
            address: PublicKey;
            seedMappings: Record<string, PublicKey>;
          } | null => {
            // Build seed buffers including const seeds in correct order
            const seedBuffers: Buffer[] = [];
            const seedMappings: Record<string, PublicKey> = {};
            let accountSeedIdx = 0;

            for (const seed of metadata.seeds) {
              if (seed.type === "const") {
                seedBuffers.push(Buffer.from(seed.value, "utf8"));
              } else {
                // Account seed - get from the combination
                if (accountSeedIdx >= seedIndices.length) return null;
                try {
                  const pk = translateAddress(
                    mainAccountEntries[
                      seedIndices[accountSeedIdx]
                    ][1] as Address
                  );
                  if (pk.equals(programId)) return null;
                  seedBuffers.push(pk.toBuffer());
                  // Track which seed name maps to which account
                  seedMappings[seed.value] = pk;
                } catch {
                  return null;
                }
                accountSeedIdx++;
              }
            }

            try {
              const [derivedPda] = PublicKey.findProgramAddressSync(
                seedBuffers,
                programId
              );

              // Check if this derived PDA matches any main instruction account
              // Skip if this address has already been claimed by another compressible account
              if (resolvedAddresses.has(derivedPda.toBase58())) {
                return null; // Already used, skip this combination
              }

              for (const [mainAccName, mainAccValue] of mainAccountEntries) {
                try {
                  const mainPubkey = translateAddress(mainAccValue as Address);
                  if (derivedPda.equals(mainPubkey)) {
                    const seedNames = seedIndices.map(
                      (i) => mainAccountEntries[i][0]
                    );
                    console.log(
                      `[decompressIfNeeded]   │  → Derived token with seeds [${seedNames.join(
                        ", "
                      )}] → ${derivedPda.toBase58()}`
                    );
                    return { address: derivedPda, seedMappings };
                  }
                } catch {}
              }
            } catch {}
            return null;
          };

          // Generate combinations
          const generateCombinations = (
            arr: number[],
            k: number
          ): number[][] => {
            if (k === 0) return [[]];
            if (arr.length === 0) return [];
            const [first, ...rest] = arr;
            const withFirst = generateCombinations(rest, k - 1).map((c) => [
              first,
              ...c,
            ]);
            const withoutFirst = generateCombinations(rest, k);
            return [...withFirst, ...withoutFirst];
          };

          const accountIndices = Array.from(
            { length: mainAccountEntries.length },
            (_, i) => i
          );
          const combinations = generateCombinations(
            accountIndices,
            expectedSeedCount
          );

          console.log(
            `[decompressIfNeeded]   │  Trying ${combinations.length} token seed combinations...`
          );

          for (const combo of combinations) {
            const result = tryDerivation(combo);
            if (result) {
              resolved = result.address;
              resolvedAddresses.add(result.address.toBase58());

              // Store the seed mappings in completeAccounts
              for (const [seedName, seedValue] of Object.entries(
                result.seedMappings
              )) {
                if (!(seedName in completeAccounts)) {
                  completeAccounts[seedName] = seedValue;
                  console.log(
                    `[decompressIfNeeded]   │  → Collected seed: '${seedName}' = ${seedValue.toBase58()}`
                  );
                }
              }

              console.log(
                `[decompressIfNeeded]   └─ ✓ P4 Brute-force: found match for token '${accountName}'`
              );
              break;
            }
          }
        } else if (
          metadata.accountType === "pda" &&
          metadata.seeds.length > 0
        ) {
          // For PDA accounts, try all seed combinations
          console.log(
            `[decompressIfNeeded]   ├─ PDA account: trying seed combinations...`
          );

          const accountSeeds = metadata.seeds.filter(
            (s) => s.type === "account"
          );
          const expectedSeedCount = accountSeeds.length;
          const seedsStr = metadata.seeds
            .map((s) => (s.type === "const" ? `"${s.value}"` : s.value))
            .join(", ");
          console.log(
            `[decompressIfNeeded]   │  Need ${expectedSeedCount} account seed(s): [${seedsStr}]`
          );

          // Try all permutations of accounts as potential seeds
          const tryDerivation = (
            seedIndices: number[]
          ): {
            address: PublicKey;
            seedMappings: Record<string, PublicKey>;
          } | null => {
            // Build seed buffers including const seeds in correct order
            const seedBuffers: Buffer[] = [];
            const seedMappings: Record<string, PublicKey> = {};
            let accountSeedIdx = 0;

            for (const seed of metadata.seeds) {
              if (seed.type === "const") {
                seedBuffers.push(Buffer.from(seed.value, "utf8"));
              } else {
                // Account seed - get from the combination
                if (accountSeedIdx >= seedIndices.length) return null;
                try {
                  const pk = translateAddress(
                    mainAccountEntries[
                      seedIndices[accountSeedIdx]
                    ][1] as Address
                  );
                  if (pk.equals(programId)) return null;
                  seedBuffers.push(pk.toBuffer());
                  // Track which seed name maps to which account
                  seedMappings[seed.value] = pk;
                } catch {
                  return null;
                }
                accountSeedIdx++;
              }
            }

            try {
              const [derivedPda] = PublicKey.findProgramAddressSync(
                seedBuffers,
                programId
              );

              // Check if this derived PDA matches any main instruction account
              // Skip if this address has already been claimed by another compressible account
              if (resolvedAddresses.has(derivedPda.toBase58())) {
                return null; // Already used, skip this combination
              }

              for (const [mainAccName, mainAccValue] of mainAccountEntries) {
                try {
                  const mainPubkey = translateAddress(mainAccValue as Address);
                  if (derivedPda.equals(mainPubkey)) {
                    const seedNames = seedIndices.map(
                      (i) => mainAccountEntries[i][0]
                    );
                    console.log(
                      `[decompressIfNeeded]   │  → Derived PDA with seeds [${seedNames.join(
                        ", "
                      )}] → ${derivedPda.toBase58()}`
                    );
                    return { address: derivedPda, seedMappings };
                  }
                } catch {}
              }
            } catch {}
            return null;
          };

          // Generate combinations of seed indices
          const generateCombinations = (
            arr: number[],
            k: number
          ): number[][] => {
            if (k === 0) return [[]];
            if (arr.length === 0) return [];
            const [first, ...rest] = arr;
            const withFirst = generateCombinations(rest, k - 1).map((c) => [
              first,
              ...c,
            ]);
            const withoutFirst = generateCombinations(rest, k);
            return [...withFirst, ...withoutFirst];
          };

          const accountIndices = Array.from(
            { length: mainAccountEntries.length },
            (_, i) => i
          );
          const combinations = generateCombinations(
            accountIndices,
            expectedSeedCount
          );

          console.log(
            `[decompressIfNeeded]   │  Trying ${combinations.length} PDA seed combinations...`
          );

          for (const combo of combinations) {
            const result = tryDerivation(combo);
            if (result) {
              resolved = result.address;
              resolvedAddresses.add(result.address.toBase58());

              // Store the seed mappings in completeAccounts
              for (const [seedName, seedValue] of Object.entries(
                result.seedMappings
              )) {
                if (!(seedName in completeAccounts)) {
                  completeAccounts[seedName] = seedValue;
                  console.log(
                    `[decompressIfNeeded]   │  → Collected seed: '${seedName}' = ${seedValue.toBase58()}`
                  );
                }
              }

              console.log(
                `[decompressIfNeeded]   └─ ✓ P4 Brute-force: found match for PDA '${accountName}'`
              );
              break;
            }
          }
        } else {
          // No seeds defined - try exact name matching only (no fuzzy matching to avoid false positives)
          console.log(
            `[decompressIfNeeded]   ├─ No seeds - trying exact name matching only...`
          );

          for (const [mainAccName, mainAccValue] of mainAccountEntries) {
            try {
              const mainPubkey = translateAddress(mainAccValue as Address);
              if (mainPubkey.equals(programId)) continue;

              const mainSnakeName = mainAccName
                .replace(/([A-Z])/g, "_$1")
                .toLowerCase()
                .replace(/^_/, "");

              // Check exact match only (no fuzzy matching to prevent false positives)
              if (mainSnakeName === accountName) {
                resolved = mainPubkey;
                console.log(
                  `[decompressIfNeeded]   └─ ✓ P4 Exact name match: '${accountName}' → '${mainAccName}' → ${mainPubkey.toBase58()}`
                );
                break;
              }
            } catch {}
          }

          // // If still not resolved after exact match, throw error
          // if (!resolved) {
          //   console.log(
          //     `[decompressIfNeeded]   ✗ Account '${accountName}' has no seeds and no exact name match found`
          //   );
          //   throw new Error(
          //     `[decompressIfNeeded] Account '${accountName}' is marked as compressible but has no seeds defined in the IDL.\n\n` +
          //       `All compressible accounts must have PDA seed definitions (enforced by the Light Protocol macro).\n\n` +
          //       `This error indicates:\n` +
          //       `  - The IDL may be malformed or corrupted\n` +
          //       `  - The program was not properly compiled with the Light Protocol macros\n` +
          //       `  - The IDL was manually edited incorrectly\n\n` +
          //       `Solutions:\n` +
          //       `  1. Rebuild the program and regenerate the IDL\n` +
          //       `  2. Provide the account explicitly: .decompressIfNeeded({ ${accountName}: <address>, ... })\n` +
          //       `  3. If this account isn't needed for this instruction, don't provide it explicitly\n\n` +
          //       `For support, visit: https://docs.lightprotocol.com`
          //   );
          // }
        }

        if (!resolved) {
          console.log(
            `[decompressIfNeeded]   └─ ✗ P4 Brute-force: no match found`
          );
        }
      }

      if (resolved) {
        completeAccounts[accountName] = resolved;
        const ataDer = this._ataDerivations[accountName];
        compressibleAccountsToCheck.push({
          name: accountName,
          address: resolved,
          ataDerivation: ataDer,
        });
        resolvedAddresses.add(resolved.toBase58()); // Mark this address as claimed
        console.log(
          `[decompressIfNeeded] ✓✓ RESOLVED '${accountName}' → ${resolved.toBase58()}`
        );

        // IMPORTANT: Now collect the seeds for this account (if any)
        // Seeds are required accounts in the decompress instruction
        if (metadata.seeds.length > 0) {
          console.log(
            `[decompressIfNeeded]   → Collecting seeds for '${accountName}'...`
          );
          for (const seed of metadata.seeds) {
            if (seed.type === "account") {
              const seedName = seed.value;

              // Skip if we already collected this seed
              if (seedName in completeAccounts) {
                console.log(
                  `[decompressIfNeeded]   │  ✓ seed '${seedName}': already collected`
                );
                continue;
              }

              let seedValue: PublicKey | null = null;

              // Try to resolve the seed account
              // 1. Check if explicitly provided
              if (
                this._decompressAccounts &&
                seedName in this._decompressAccounts
              ) {
                try {
                  const provided = translateAddress(
                    this._decompressAccounts[seedName] as Address
                  );
                  if (!provided.equals(programId)) {
                    seedValue = provided;
                    console.log(
                      `[decompressIfNeeded]   │  ✓ seed '${seedName}': explicit → ${seedValue.toBase58()}`
                    );
                  }
                } catch {}
              }

              // 2. Check if in main instruction
              if (!seedValue) {
                seedValue = this._getFromMainInstruction(seedName);
                if (seedValue) {
                  console.log(
                    `[decompressIfNeeded]   │  ✓ seed '${seedName}': from main → ${seedValue.toBase58()}`
                  );
                }
              }

              if (!seedValue && this._accounts) {
                const seedNameLower = seedName.toLowerCase();
                const seedNameSnake = seedName
                  .replace(/([A-Z])/g, "_$1")
                  .toLowerCase()
                  .replace(/^_/, "");

                for (const [mainAccName, mainAccValue] of Object.entries(
                  this._accounts
                )) {
                  try {
                    const mainPubkey = translateAddress(
                      mainAccValue as Address
                    );
                    if (mainPubkey.equals(programId)) continue;

                    const mainNameLower = mainAccName.toLowerCase();
                    const mainNameSnake = mainAccName
                      .replace(/([A-Z])/g, "_$1")
                      .toLowerCase()
                      .replace(/^_/, "");

                    // Strategy 1: Exact match (case-insensitive)
                    if (seedNameLower === mainNameLower) {
                      seedValue = mainPubkey;
                      console.log(
                        `[decompressIfNeeded]   │  ✓ seed '${seedName}': case-insensitive match '${mainAccName}' → ${seedValue.toBase58()}`
                      );
                      break;
                    }

                    // Strategy 2: Snake case equivalents match
                    if (seedNameSnake === mainNameSnake) {
                      seedValue = mainPubkey;
                      console.log(
                        `[decompressIfNeeded]   │  ✓ seed '${seedName}': snake_case match '${mainAccName}' → ${seedValue.toBase58()}`
                      );
                      break;
                    }
                  } catch {}
                }
              }

              // 4. Check if already collected in completeAccounts
              if (!seedValue && seedName in completeAccounts) {
                seedValue = completeAccounts[seedName];
                console.log(
                  `[decompressIfNeeded]   │  ✓ seed '${seedName}': found in completeAccounts → ${seedValue.toBase58()}`
                );
              }

              // 5. Try brute-force mapping of unresolved seeds by deriving the target account
              if (!seedValue && resolved) {
                // Build knownSeedValues for this account only
                const knownSeedValues: Record<string, PublicKey> = {};
                for (const s of metadata.seeds) {
                  if (s.type === "account") {
                    const nm = s.value;
                    if (nm in completeAccounts) {
                      try {
                        knownSeedValues[nm] = translateAddress(
                          completeAccounts[nm] as Address
                        );
                      } catch {
                        // already PublicKey
                        knownSeedValues[nm] = completeAccounts[nm];
                      }
                    }
                  }
                }

                const mapping = this._bruteForceResolveSeedAccounts(
                  accountName,
                  metadata,
                  resolved,
                  knownSeedValues
                );

                if (mapping) {
                  for (const [nm, pk] of Object.entries(mapping)) {
                    if (!(nm in completeAccounts)) {
                      completeAccounts[nm] = pk;
                      console.log(
                        `[decompressIfNeeded]   │  ✓ seed '${nm}': brute-force → ${pk.toBase58()}`
                      );
                    }
                  }
                  if (seedName in mapping) {
                    seedValue = mapping[seedName];
                  }
                }
              }

              // 6. REQUIRED: All seeds for resolved compressible accounts MUST be found
              if (!seedValue) {
                // Log what we have to help debugging
                console.log(
                  `[decompressIfNeeded]   │  ✗ seed '${seedName}': NOT FOUND`
                );
                console.log(
                  `[decompressIfNeeded]   │  Available main instruction accounts: ${
                    Object.keys(this._accounts || {}).join(", ") || "none"
                  }`
                );
                throw new Error(
                  `[decompressIfNeeded] Compressible account '${accountName}' is being decompressed, ` +
                    `but its required seed account '${seedName}' could not be resolved.\n\n` +
                    `This account is required because '${accountName}' is explicitly provided or found in the instruction.\n\n` +
                    `Solutions:\n` +
                    `  1. Provide it explicitly: .decompressIfNeeded({ ${seedName}: <publicKey>, ... })\n` +
                    `  2. Include it in the main instruction accounts\n` +
                    `  3. If '${accountName}' is not needed for this instruction, don't provide it explicitly\n\n` +
                    `Note: '${seedName}' may be a field reference - ensure the field's value is passed as a PublicKey.`
                );
              }
              completeAccounts[seedName] = seedValue;
            }
          }
        }
      } else {
        // Not resolved - treat as unused (set to program ID / None)
        console.log(
          `[decompressIfNeeded] ✗✗ FAILED to resolve '${accountName}' - treating as unused (None)`
        );
        completeAccounts[accountName] = programId;
      }
    }

    // Step 3: Process remaining non-compressible decompress accounts (system accounts, optional accounts)
    for (const acc of decompressInstruction.accounts || []) {
      const accountName = typeof acc === "string" ? acc : acc.name;
      const isOptional =
        typeof acc !== "string" &&
        !("accounts" in acc) &&
        (acc as IdlInstructionAccount).optional === true;
      const isSystemAccount = SYSTEM_ACCOUNTS.has(accountName);

      // Skip if already resolved (either compressible account or collected seed)
      if (accountName in completeAccounts) {
        console.log(
          `[decompressIfNeeded] ✓ '${accountName}' already resolved (collected from seed resolution):`,
          completeAccounts[accountName].toBase58()
        );
        continue;
      }

      let resolved: PublicKey | null = null;

      // Check explicit value
      if (accountName in this._decompressAccounts) {
        try {
          const provided = translateAddress(
            this._decompressAccounts[accountName] as Address
          );
          if (!provided.equals(programId)) {
            resolved = provided;
            console.log(
              `[decompressIfNeeded] Using explicit value for '${accountName}':`,
              provided.toBase58()
            );
          }
        } catch {}
      }

      // Check main instruction
      if (!resolved) {
        const fromMain = this._getFromMainInstruction(accountName);
        if (fromMain) {
          console.log(
            `[decompressIfNeeded] Auto-pulling '${accountName}' from main instruction:`,
            fromMain.toBase58()
          );
          resolved = fromMain;
        }
      }

      // Set to programId (None) if still not resolved
      if (!resolved) {
        if (!isOptional && !isSystemAccount) {
          throw new Error(
            `[decompressIfNeeded] Required account '${accountName}' is missing.\n` +
              `Please provide it explicitly: .decompressIfNeeded({ ..., ${accountName}: <publicKey> })`
          );
        }
        console.log(
          `[decompressIfNeeded] Account '${accountName}' not found - treating as unused (None)`
        );
        resolved = programId;
      }

      completeAccounts[accountName] = resolved;
    }

    console.log(
      `[decompressIfNeeded] Resolved ${
        Object.keys(completeAccounts).length
      } total accounts for decompress instruction`
    );
    console.log(
      `[decompressIfNeeded] Will check compression state for ${compressibleAccountsToCheck.length} resolved compressible account(s):`
    );
    compressibleAccountsToCheck.forEach((acc, idx) => {
      console.log(`  [${idx + 1}] ${acc.name} → ${acc.address.toBase58()}`);
    });

    // Step 4: Fetch compression state for all potentially compressible accounts
    const accountInputs = await this._fetchCompressibleAccounts(
      compressibleAccountsToCheck
    );

    if (accountInputs.length === 0) {
      console.log(
        "[decompressIfNeeded] No compressed accounts found. Skipping decompress."
      );
      return;
    }

    // Step 5: Build decompress params
    const rpc = createRpc();
    console.log(
      `[decompressIfNeeded] Building decompress params for ${accountInputs.length} compressed account(s)`
    );

    console.log(
      "accountInputs",
      accountInputs.map((acc) => acc)
    );
    const params = await buildDecompressParams(programId, rpc, accountInputs);

    console.log("params", params);
    console.log(
      "params",
      params?.compressedAccounts.map((acc) => JSON.stringify(acc))
    );

    if (!params) {
      console.log("[decompressIfNeeded] buildDecompressParams returned null.");
      return;
    }

    console.log(
      `[decompressIfNeeded] Built params with ${params.compressedAccounts.length} compressed account(s), ${params.remainingAccounts.length} remaining account(s)`
    );

    // Step 6: Build and inject the decompress instruction
    const decompressIxFn = this._allInstructionFns[
      "decompressAccountsIdempotent"
    ] as InstructionFn<IDL, DecompressInstruction<IDL>>;

    if (!decompressIxFn) {
      console.warn(
        "[decompressIfNeeded] decompressAccountsIdempotent instruction function not found"
      );
      return;
    }

    let decompressIx;
    try {
      decompressIx = decompressIxFn(
        params.proofOption,
        params.compressedAccounts,
        params.systemAccountsOffset,
        {
          accounts: completeAccounts as Accounts<D>,
          remainingAccounts: params.remainingAccounts,
        }
      );
    } catch (error) {
      console.error(
        `[decompressIfNeeded] ERROR building decompress instruction:`,
        (error as Error).message
      );
      throw error;
    }

    console.log(
      `[decompressIfNeeded] SUCCESS! Decompress instruction built. Prepending to transaction.`
    );

    // Prepend decompress instruction
    this.preInstructions([decompressIx], true);
  }

  /**
   * Resolve a compressible account using Light metadata seed dependencies.
   * Tries to find seed accounts in main instruction and derive the target account.
   */
  private _resolveCompressibleAccountFromMetadata(
    accountName: string,
    metadata: {
      accountType: "pda" | "token";
      seeds: Array<{ type: "const" | "account"; value: string }>;
    }
  ): {
    found: boolean;
    address?: PublicKey;
    seeds?: Record<string, PublicKey>;
  } {
    const programId = this._programId;
    const seedsResolved: Record<string, PublicKey> = {};
    const seedBuffers: Buffer[] = [];

    // Try to resolve all seeds (both const and account)
    const accountSeeds = metadata.seeds.filter((s) => s.type === "account");
    console.log(
      `[decompressIfNeeded]   ├─ Resolving ${accountSeeds.length} account seed(s) for '${accountName}'...`
    );

    // Process each seed in order (preserving const and account seeds)
    for (const seed of metadata.seeds) {
      if (seed.type === "const") {
        // Const seed - add directly to seed buffers
        seedBuffers.push(Buffer.from(seed.value, "utf8"));
        console.log(`[decompressIfNeeded]   │  ✓ const seed: "${seed.value}"`);
      } else {
        // Account seed - need to resolve it
        const seedName = seed.value;
        let seedValue: PublicKey | null = null;

        // Check if seed is explicitly provided
        if (this._decompressAccounts && seedName in this._decompressAccounts) {
          try {
            const provided = translateAddress(
              this._decompressAccounts[seedName] as Address
            );
            if (!provided.equals(programId)) {
              seedValue = provided;
              console.log(
                `[decompressIfNeeded]   │  ✓ account seed '${seedName}': explicit → ${seedValue.toBase58()}`
              );
            }
          } catch {}
        }

        // Check if seed is in main instruction
        if (!seedValue) {
          seedValue = this._getFromMainInstruction(seedName);
          if (seedValue) {
            console.log(
              `[decompressIfNeeded]   │  ✓ account seed '${seedName}': from main → ${seedValue.toBase58()}`
            );
          }
        }

        if (!seedValue) {
          console.log(
            `[decompressIfNeeded]   │  ✗ account seed '${seedName}': NOT FOUND`
          );
          return { found: false };
        }

        seedsResolved[seedName] = seedValue;
        seedBuffers.push(seedValue.toBuffer());
      }
    }

    // All seeds found - now derive the account
    console.log(
      `[decompressIfNeeded]   ├─ All seeds resolved. Deriving ${metadata.accountType}...`
    );

    try {
      if (metadata.accountType === "pda") {
        // Derive PDA using all seed buffers (const + account)
        const [derivedPda] = PublicKey.findProgramAddressSync(
          seedBuffers,
          programId
        );
        console.log(
          `[decompressIfNeeded]   │  → Derived PDA: ${derivedPda.toBase58()}`
        );

        // Check if this matches any account in main instruction
        if (this._accounts) {
          for (const [mainAccName, mainAccValue] of Object.entries(
            this._accounts
          )) {
            try {
              const mainPubkey = translateAddress(mainAccValue as Address);
              if (mainPubkey.equals(derivedPda)) {
                console.log(
                  `[decompressIfNeeded]   └─ ✓ Match found: derived PDA equals main account '${mainAccName}'`
                );
                return {
                  found: true,
                  address: derivedPda,
                  seeds: seedsResolved,
                };
              }
            } catch {}
          }
        }

        // Even if not in main instruction, we successfully derived it
        console.log(
          `[decompressIfNeeded]   └─ ✓ PDA derived (not found in main instruction, but usable)`
        );
        return { found: true, address: derivedPda, seeds: seedsResolved };
      } else if (metadata.accountType === "token") {
        // For token accounts (compressed token vaults), derive the actual PDA
        // Token vaults are PDAs: PDA[const_seed, ...account_seeds]
        const [derivedTokenVault] = PublicKey.findProgramAddressSync(
          seedBuffers,
          programId
        );
        console.log(
          `[decompressIfNeeded]   │  → Derived token vault PDA: ${derivedTokenVault.toBase58()}`
        );

        // Check if this derived address matches any account in main instruction
        if (this._accounts) {
          for (const [mainAccName, mainAccValue] of Object.entries(
            this._accounts
          )) {
            try {
              const mainPubkey = translateAddress(mainAccValue as Address);
              if (mainPubkey.equals(derivedTokenVault)) {
                console.log(
                  `[decompressIfNeeded]   └─ ✓ Match found: derived token vault equals main account '${mainAccName}'`
                );
                return {
                  found: true,
                  address: derivedTokenVault,
                  seeds: seedsResolved,
                };
              }
            } catch {}
          }
        }

        // Token vault not found in main instruction - this is expected if it's not used
        console.log(
          `[decompressIfNeeded]   └─ ✗ Derived token vault not found in main instruction`
        );
      }
    } catch (error) {
      console.log(
        `[decompressIfNeeded]   └─ ✗ Derivation failed: ${
          (error as Error).message
        }`
      );
    }

    return { found: false };
  }

  /**
   * Brute-force resolution for missing seed accounts by deriving the target account
   * using candidate assignments for unresolved seeds and matching against the
   * already-resolved target address.
   */
  private _bruteForceResolveSeedAccounts(
    accountName: string,
    metadata: {
      accountType: "pda" | "token";
      seeds: Array<{ type: "const" | "account"; value: string }>;
    },
    targetAddress: PublicKey,
    knownSeedValues: Record<string, PublicKey>
  ): Record<string, PublicKey> | null {
    const programId = this._programId;

    // Collect unresolved seed names (in order of appearance)
    const unresolvedSeeds: string[] = [];
    for (const seed of metadata.seeds) {
      if (seed.type === "account") {
        const name = seed.value;
        if (!knownSeedValues[name]) {
          unresolvedSeeds.push(name);
        }
      }
    }

    if (unresolvedSeeds.length === 0) {
      return {};
    }

    const mainAccountEntries = Object.entries(this._accounts || {});

    // Build candidate indices from main accounts
    const candidateIndices: number[] = [];
    for (let i = 0; i < mainAccountEntries.length; i++) {
      try {
        const pk = translateAddress(mainAccountEntries[i][1] as Address);
        // Exclude placeholders (program id)
        if (!pk.equals(programId)) {
          candidateIndices.push(i);
        }
      } catch {}
    }

    // Helper: generate permutations (ordered selections without repetition)
    const generatePermutations = (arr: number[], k: number): number[][] => {
      if (k === 0) return [[]];
      const results: number[][] = [];
      const backtrack = (path: number[], used: boolean[]) => {
        if (path.length === k) {
          results.push(path.slice());
          return;
        }
        for (let idx = 0; idx < arr.length; idx++) {
          if (used[idx]) continue;
          used[idx] = true;
          path.push(arr[idx]);
          backtrack(path, used);
          path.pop();
          used[idx] = false;
        }
      };
      backtrack([], new Array(arr.length).fill(false));
      return results;
    };

    const permutations = generatePermutations(
      candidateIndices,
      unresolvedSeeds.length
    );

    // For each permutation, assign candidates to unresolved seeds (in order),
    // build the seed buffers and derive. If it matches the target address, we
    // found the mapping.
    let foundMapping: Record<string, PublicKey> | null = null;
    for (const perm of permutations) {
      const assignment: Record<string, PublicKey> = {};
      let unresolvedIdx = 0;
      const seedBuffers: Buffer[] = [];

      let valid = true;
      for (const seed of metadata.seeds) {
        if (seed.type === "const") {
          seedBuffers.push(Buffer.from(seed.value, "utf8"));
        } else {
          const name = seed.value;
          let valuePk: PublicKey | null = null;
          if (knownSeedValues[name]) {
            valuePk = knownSeedValues[name];
          } else {
            // Assign from permutation
            const candIndex = perm[unresolvedIdx++];
            try {
              valuePk = translateAddress(
                mainAccountEntries[candIndex][1] as Address
              );
            } catch {
              valid = false;
            }
            if (!valuePk) valid = false;
            if (!valid) break;
            assignment[name] = valuePk!;
          }
          seedBuffers.push(valuePk!.toBuffer());
        }
      }
      if (!valid) continue;

      try {
        const [derived] = PublicKey.findProgramAddressSync(
          seedBuffers,
          programId
        );
        if (derived.equals(targetAddress)) {
          if (foundMapping) {
            // Ambiguous mapping - multiple solutions. Bail to avoid false positives.
            return null;
          }
          foundMapping = assignment;
        }
      } catch {}
    }

    return foundMapping;
  }

  /**
   * Create an instruction based on the current configuration.
   *
   * See {@link transaction} to create a transaction instead.
   *
   * @returns the transaction instruction
   */
  public async instruction(): Promise<TransactionInstruction> {
    if (this._enableAutoDecompress) {
      throw new Error(
        "decompressIfNeeded() cannot be used with .instruction(). " +
          "Use .transaction() or .rpc() instead, which handle multiple instructions."
      );
    }

    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    // @ts-ignore
    return this._ixFn(...this._args, {
      accounts: this._accounts,
      signers: this._signers,
      remainingAccounts: this._remainingAccounts,
      preInstructions: this._preInstructions,
      postInstructions: this._postInstructions,
    });
  }

  /**
   * Create a transaction based on the current configuration.
   *
   * This method doesn't send the created transaction. Use {@link rpc} method
   * to conveniently send an confirm the configured transaction.
   *
   * See {@link instruction} to only create an instruction instead.
   *
   * @returns the transaction
   */
  public async transaction(): Promise<Transaction> {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    // Inject decompress if configured
    await this._injectDecompressIfNeeded();

    // @ts-ignore
    return this._txFn(...this._args, {
      accounts: this._accounts,
      signers: this._signers,
      remainingAccounts: this._remainingAccounts,
      preInstructions: this._preInstructions,
      postInstructions: this._postInstructions,
    });
  }

  /**
   * Simulate the configured transaction.
   *
   * @param options confirmation options
   * @returns the simulation response
   */
  public async simulate(options?: ConfirmOptions): Promise<SimulateResponse> {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    // @ts-ignore
    return this._simulateFn(...this._args, {
      accounts: this._accounts,
      signers: this._signers,
      remainingAccounts: this._remainingAccounts,
      preInstructions: this._preInstructions,
      postInstructions: this._postInstructions,
      options,
    });
  }

  /**
   * View the configured transaction.
   *
   * Note that to use this method, the instruction needs to return a value and
   * all its accounts must be read-only.
   *
   * @param options confirmation options
   * @returns the return value of the instruction
   */
  public async view(options?: ConfirmOptions): Promise<any> {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    if (!this._viewFn) {
      throw new Error(
        [
          "Method does not support views.",
          "The instruction should return a value, and its accounts must be read-only",
        ].join(" ")
      );
    }

    // @ts-ignore
    return this._viewFn(...this._args, {
      accounts: this._accounts,
      signers: this._signers,
      remainingAccounts: this._remainingAccounts,
      preInstructions: this._preInstructions,
      postInstructions: this._postInstructions,
      options,
    });
  }

  /**
   * Send and confirm the configured transaction.
   *
   * See {@link rpcAndKeys} to both send the transaction and get the resolved
   * account public keys.
   *
   * @param options confirmation options
   * @returns the transaction signature
   */
  public async rpc(options?: ConfirmOptions): Promise<TransactionSignature> {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    // Inject decompress if configured
    await this._injectDecompressIfNeeded();

    // @ts-ignore
    return this._rpcFn(...this._args, {
      accounts: this._accounts,
      signers: this._signers,
      remainingAccounts: this._remainingAccounts,
      preInstructions: this._preInstructions,
      postInstructions: this._postInstructions,
      options,
    });
  }

  /**
   * Conveniently call both {@link rpc} and {@link pubkeys} methods.
   *
   * @param options confirmation options
   * @returns the transaction signature and account public keys
   */
  public async rpcAndKeys(options?: ConfirmOptions): Promise<{
    signature: TransactionSignature;
    pubkeys: InstructionAccountAddresses<IDL, I>;
  }> {
    return {
      signature: await this.rpc(options),
      pubkeys: (await this.pubkeys()) as Required<
        InstructionAccountAddresses<IDL, I>
      >,
    };
  }

  /**
   * Get instruction information necessary to include the instruction inside a
   * transaction.
   *
   * # Example
   *
   * ```ts
   * const { instruction, signers, pubkeys } = await method.prepare();
   * ```
   */
  public async prepare(): Promise<{
    instruction: TransactionInstruction;
    signers: Signer[];
    pubkeys: Partial<InstructionAccountAddresses<IDL, I>>;
  }> {
    return {
      instruction: await this.instruction(),
      signers: this._signers,
      pubkeys: await this.pubkeys(),
    };
  }
}
