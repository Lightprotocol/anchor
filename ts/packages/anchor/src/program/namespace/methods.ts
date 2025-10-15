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
  CTOKEN_RENT_SPONSOR,
} from "@lightprotocol/compressed-token";

featureFlags.version = VERSION.V2;
/**
 * Infer account type and variant from IDL for compression.
 * Returns { accountType, tokenVariant? } based STRICTLY on IDL definitions.
 * NO FALLBACKS OR HEURISTICS - if not found in IDL, returns null.
 */
function inferAccountTypeFromIDL(
  accountName: string,
  idl: Idl | undefined,
  idlTypes: IdlTypeDef[]
): { accountType: string; tokenVariant?: string } | null {
  if (!idl) return null;

  // 1. Check if it's a named program account in idl.accounts
  const accountDef = idl.accounts?.find((acc) => acc.name === accountName);
  if (accountDef) {
    return { accountType: accountName };
  }

  // 2. Check if it matches a compressed token variant enum
  // Look for the enum type that defines token variants (e.g., cTokenAccountVariant)
  const compressedVariantType = idlTypes.find(
    (t) =>
      t.name.toLowerCase().includes("compressedaccountvariant") ||
      t.name.toLowerCase().includes("ctokenaccountvariant") ||
      t.name.toLowerCase().includes("tokenvariant")
  );

  if (compressedVariantType && compressedVariantType.type.kind === "enum") {
    const variants = compressedVariantType.type.variants ?? [];
    const matchedVariant = variants.find(
      (v) => v.name.toLowerCase() === accountName.toLowerCase()
    );

    if (matchedVariant) {
      return {
        accountType: "cTokenData",
        tokenVariant: matchedVariant.name,
      };
    }
  }

  // NOT FOUND IN IDL - return null (don't guess!)
  return null;
}

export type MethodsNamespace<
  IDL extends Idl = Idl,
  I extends AllInstructions<IDL> = AllInstructions<IDL>
> = MakeMethodsNamespace<IDL, I>;

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

// Scan the IDL for compressible accounts using explicit tags in account docs.
// - "cpda" tag indicates a compressible program-derived account
// - "cctoken" tag indicates a compressible token account (CompressedTokenProgram)
function scanCompressibleNames(idl: Idl | undefined): {
  cpda: Set<string>;
  cctoken: Set<string>;
} {
  const cpda = new Set<string>();
  const cctoken = new Set<string>();
  if (!idl) return { cpda, cctoken };

  const visitAccounts = (items: readonly IdlInstructionAccountItem[]) => {
    for (const item of items) {
      if ("accounts" in item) {
        visitAccounts(item.accounts);
        continue;
      }
      const acc = item as IdlInstructionAccount;
      const docs = (acc.docs || []).map((d) => d.toLowerCase());
      if (docs.some((d) => d.includes("cctoken"))) {
        cctoken.add(acc.name);
      }
      if (docs.some((d) => d.includes("cpda"))) {
        cpda.add(acc.name);
      }
    }
  };

  for (const ix of idl.instructions) {
    visitAccounts(ix.accounts);
  }

  return { cpda, cctoken };
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
  private _idlTypes: IdlTypeDef[];
  private _allInstructionFns?: Record<string, InstructionFn<IDL>>;
  private _programId: PublicKey;
  private _accountNamespace: AccountNamespace<IDL>;

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
   * Enable automatic decompression for compressible accounts.
   *
   * Requires ALL accounts needed by decompressAccountsIdempotent instruction to be provided.
   * Automatically infers account types and variants from IDL, fetches compression state,
   * and injects decompress instruction if needed.
   *
   * @param accounts - All accounts for decompressAccountsIdempotent instruction.
   * @returns this builder for chaining
   *
   * @example
   * ```ts
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded({
   *     feePayer: owner.publicKey,
   *     config: compressionConfig,
   *     rentPayer: owner.publicKey,
   *     ctokenRentSponsor: CTOKEN_RENT_SPONSOR,
   *     ctokenProgram: CompressedTokenProgram.programId,
   *     ctokenCpiAuthority: CompressedTokenProgram.deriveCpiAuthorityPda,
   *     ctokenConfig,
   *     ammConfig,
   *     poolState,
   *     token0Mint,
   *     token1Mint,
   *     lpMint,
   *     token0Vault,
   *     token1Vault,
   *     lpVault,
   *   })
   *   .accounts({ ... })
   *   .rpc();
   * ```
   */
  public decompressIfNeeded(
    accounts: Accounts<D> & { [name: string]: Address }
  ) {
    this._enableAutoDecompress = true;
    this._decompressAccounts = flattenPartialAccounts(accounts as any, true);
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
   */
  private async _fetchCompressibleAccounts(
    accountsToCheck: Array<{ name: string; address: PublicKey }>
  ): Promise<AccountInput[]> {
    if (accountsToCheck.length === 0) return [];

    const programId = this._programId;
    const rpc = createRpc();

    const defaultAddressTreeInfo = getDefaultAddressTreeInfo();

    const accountInputs: AccountInput[] = [];

    // Determine compressible names by tags once
    const tags = scanCompressibleNames(this._idl);
    for (const { name, address } of accountsToCheck) {
      // Only consider accounts explicitly tagged in IDL
      const isCpda = tags.cpda.has(name);
      const isCCToken = tags.cctoken.has(name);
      if (!isCpda && !isCCToken) {
        console.log(
          `[decompressIfNeeded] Skipping account '${name}' (no 'cpda' or 'cctoken' tag)`
        );
        continue;
      }

      // Try fetching as CPDA
      // @ts-ignore
      const cpdaResult = await rpc.getAccountInfoInterface(
        address,
        programId,
        //@ts-ignore
        defaultAddressTreeInfo
      );

      if (cpdaResult && cpdaResult.merkleContext && cpdaResult.isCompressed) {
        // It's a compressed CPDA
        if (
          isCCToken ||
          cpdaResult.accountInfo.owner.equals(CompressedTokenProgram.programId)
        ) {
          // It's a compressed token account
          try {
            const parsed = parseTokenData(cpdaResult.accountInfo.data);
            if (parsed) {
              accountInputs.push({
                address,
                info: {
                  accountInfo: cpdaResult.accountInfo,
                  parsed,
                  merkleContext: cpdaResult.merkleContext,
                },
                accountType: "cTokenData",
                tokenVariant: name,
              });
            }
          } catch {
            // Not a token account, ignore
          }
        } else {
          // It's a compressed program account
          let parsed = null;
          const accountClient = (this._accountNamespace as any)[name];
          if (accountClient?.coder) {
            try {
              parsed = accountClient.coder.accounts.decode(
                name,
                cpdaResult.accountInfo.data
              );
            } catch {
              try {
                parsed = accountClient.coder.accounts.decodeAny(
                  cpdaResult.accountInfo.data
                );
              } catch {
                // Parsing failed, use null
              }
            }
          }

          accountInputs.push({
            address,
            info: {
              accountInfo: cpdaResult.accountInfo,
              parsed,
              merkleContext: cpdaResult.merkleContext,
            },
            accountType: name, // cpda accountType equals the IDL account name
          });
        }
        continue;
      }

      // Fallback: try as compressed token via getAccountInterface
      if (isCCToken) {
        try {
          const tokenRes = await getAccountInterface(
            rpc,
            address,
            undefined,
            CompressedTokenProgram.programId
          );
          if (tokenRes && (tokenRes as any).merkleContext) {
            accountInputs.push({
              address,
              info: tokenRes as any,
              accountType: "cTokenData",
              tokenVariant: name,
            });
          }
        } catch {
          // Not a compressed token
        }
      }
    }

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
   * Internal method to inject decompress instruction if needed.
   */
  private async _injectDecompressIfNeeded(): Promise<void> {
    if (!this._enableAutoDecompress || !this._decompressAccounts) return;

    if (!this._idl || !this._allInstructionFns) {
      console.warn(
        "[decompressIfNeeded] Missing IDL or instruction functions."
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

    // Known non-compressible system accounts to skip
    const SKIP_ACCOUNTS = new Set([
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

    // Collect accounts to check for compression
    const accountsToCheck: Array<{ name: string; address: PublicKey }> = [];
    for (const [name, addr] of Object.entries(this._decompressAccounts)) {
      if (SKIP_ACCOUNTS.has(name)) continue;

      try {
        const pubkey = translateAddress(addr as Address);
        accountsToCheck.push({ name, address: pubkey });
      } catch {
        console.warn(
          `[decompressIfNeeded] Invalid address for ${name}, skipping`
        );
      }
    }

    // Fetch compression state for all accounts
    const accountInputs = await this._fetchCompressibleAccounts(
      accountsToCheck
    );

    if (accountInputs.length === 0) {
      console.log(
        "[decompressIfNeeded] No compressed accounts found. Skipping decompress."
      );
      return;
    }

    // Build decompress params
    const programId = this._programId;
    const rpc = createRpc();

    console.log(
      `[decompressIfNeeded] Building decompress params for ${accountInputs.length} compressed account(s)`
    );
    console.log(
      `[decompressIfNeeded] Calling buildDecompressParams with inputs:`,
      // JSON.stringify(
      accountInputs.map((ai: any) => ({
        address: ai.address.toBase58(),
        accountType: ai.accountType,
        info: ai.info,
        tokenVariant: ai.tokenVariant || undefined,
        hasMerkleContext: !!ai.info?.merkleContext,
        hasParsed: !!ai.info?.parsed,
      }))
      // null,
      // 2
      // )
    );
    const params = await buildDecompressParams(programId, rpc, accountInputs);

    if (!params) {
      console.log("[decompressIfNeeded] buildDecompressParams returned null.");
      return;
    }

    console.log(
      `[decompressIfNeeded] Built params with ${params.compressedAccounts.length} compressed account(s), ${params.remainingAccounts.length} remaining account(s)`
    );
    console.log(
      `[decompressIfNeeded] compressedAccounts:`,
      JSON.stringify(params.compressedAccounts, null, 2)
    );
    console.log(
      `[decompressIfNeeded] systemAccountsOffset: ${params.systemAccountsOffset}`
    );

    // Unwrap array-wrapped variant data (SDK packs as arrays, Anchor IDL expects structs)
    const unwrappedCompressedAccounts = params.compressedAccounts.map(
      (acc: any) => {
        if (!acc?.data) return acc;

        const unwrappedData: any = {};
        for (const key in acc.data) {
          const value = acc.data[key];
          unwrappedData[key] =
            Array.isArray(value) && value.length === 1 ? value[0] : value;
        }

        return { ...acc, data: unwrappedData };
      }
    );

    // Build the decompress instruction
    const decompressIxFn = this._allInstructionFns[
      "decompressAccountsIdempotent"
    ] as InstructionFn<IDL, DecompressInstruction<IDL>>;

    if (!decompressIxFn) {
      console.warn(
        "[decompressIfNeeded] decompressAccountsIdempotent instruction function not found"
      );
      return;
    }

    console.log(
      `[decompressIfNeeded] Building instruction with accounts:`,
      Object.keys(this._decompressAccounts)
    );

    let decompressIx;
    try {
      decompressIx = decompressIxFn(
        params.proofOption,
        unwrappedCompressedAccounts,
        params.systemAccountsOffset,
        {
          accounts: this._decompressAccounts as Accounts<D>,
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
   * Create an instruction based on the current configuration.
   *
   * See {@link transaction} to create a transaction instead.
   *
   * @returns the transaction instruction
   */
  public async instruction(): Promise<TransactionInstruction> {
    if (this._resolveAccounts) {
      await this._accountsResolver.resolve();
    }

    // Inject decompress if configured
    await this._injectDecompressIfNeeded();

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
