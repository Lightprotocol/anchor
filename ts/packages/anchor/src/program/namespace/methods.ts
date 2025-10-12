import {
  AccountMeta,
  ConfirmOptions,
  PublicKey,
  Signer,
  Transaction,
  TransactionInstruction,
  TransactionSignature,
} from "@solana/web3.js";
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
import { createRpc, type Rpc } from "@lightprotocol/stateless.js";
import {
  buildDecompressParams,
  getAccountInterface,
} from "@lightprotocol/compressed-token";

/**
 * Check if an IDL type has compression_info field (is compressible)
 */
function isCompressibleType(typeName: string, idlTypes: IdlTypeDef[]): boolean {
  const type = idlTypes.find((t) => t.name === typeName);
  if (!type) return false;

  if (type.type.kind === "struct") {
    return (
      type.type.fields?.some(
        (f) => f.name === "compression_info" || f.name === "compressionInfo"
      ) ?? false
    );
  }

  // For enums, check if any variant has compression_info
  if (type.type.kind === "enum") {
    return (
      type.type.variants?.some((v) =>
        v.fields?.some(
          (f) =>
            (f as any).name === "compression_info" ||
            (f as any).name === "compressionInfo"
        )
      ) ?? false
    );
  }

  return false;
}

/**
 * Get the defined type name from an account
 */
function getDefinedTypeName(account: IdlInstructionAccountItem): string | null {
  if (!("type" in account)) return null;
  const accType: any = (account as any).type;
  if (accType && typeof accType === "object" && "defined" in accType) {
    const defined: any = accType.defined;
    if (typeof defined === "string") return defined;
    if (defined && typeof defined === "object" && "name" in defined)
      return defined.name as string;
  }
  return null;
}

/**
 * Detect if type is CTokenData by checking if it's an enum with token-like variants
 */
function isCTokenDataType(typeName: string, idlTypes: IdlTypeDef[]): boolean {
  const type = idlTypes.find((t) => t.name === typeName);
  if (!type || type.type.kind !== "enum") return false;

  // Check if it has typical cToken variant names
  const variants = type.type.variants?.map((v) => v.name.toLowerCase()) ?? [];
  const tokenKeywords = ["vault", "token", "account", "mint"];
  return variants.some((v) => tokenKeywords.some((kw) => v.includes(kw)));
}

/**
 * Extract enum variant from account name for CTokenData
 * e.g., "lpVault" -> "lpVault", "token0Vault" -> "token0Vault"
 */
function extractTokenVariant(
  accountName: string,
  typeName: string,
  idlTypes: IdlTypeDef[]
): string | undefined {
  const type = idlTypes.find((t) => t.name === typeName);
  if (!type || type.type.kind !== "enum") return undefined;

  // Try to match account name to variant name (case-insensitive)
  const variants = type.type.variants ?? [];
  const matchedVariant = variants.find(
    (v) =>
      accountName.toLowerCase().includes(v.name.toLowerCase()) ||
      v.name.toLowerCase().includes(accountName.toLowerCase())
  );

  return matchedVariant?.name;
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
    customResolver?: CustomAccountResolver<IDL>
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
        customResolver
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

export class MethodsBuilder<
  IDL extends Idl,
  I extends AllInstructions<IDL>,
  A extends I["accounts"][number] = I["accounts"][number]
> {
  private _accounts: AccountsGeneric = {};
  private _remainingAccounts: Array<AccountMeta> = [];
  private _signers: Array<Signer> = [];
  private _preInstructions: Array<TransactionInstruction> = [];
  private _postInstructions: Array<TransactionInstruction> = [];
  private _accountsResolver: AccountsResolver<IDL>;
  private _resolveAccounts: boolean = true;
  private _enableAutoDecompress: boolean = false;

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
    customResolver?: CustomAccountResolver<IDL>
  ) {
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
   * Enable automatic decompression for all compressible accounts.
   *
   * Automatically detects compressible accounts from IDL, fetches their data,
   * and injects decompress instructions if any are compressed.
   *
   * No configuration required - everything is derived from the IDL!
   *
   * @returns this builder for chaining
   *
   * @example
   * ```ts
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded()  // That's it!
   *   .accounts({ poolState, lpVault, token0Vault, ... })
   *   .rpc();
   * ```
   */
  public decompressIfNeeded() {
    this._enableAutoDecompress = true;
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
   * Automatically detect and fetch compressible accounts based on IDL.
   * Returns account inputs for buildDecompressParams.
   */
  private async _detectAndFetchCompressibleAccounts(): Promise<any[]> {
    const idlIx = this._accountsResolver["_idlIx"];
    const idlTypes = this._accountsResolver["_idlTypes"];
    const provider = this._accountsResolver["_provider"] as Provider & {
      rpc?: any;
    };

    // Find all compressible account definitions from IDL
    const compressibleAccounts: Array<{
      name: string;
      address: PublicKey;
      typeName: string;
      isCToken: boolean;
      variant?: string;
    }> = [];

    // Recursively process accounts (handles nested account structures)
    const processAccounts = (
      accounts: readonly IdlInstructionAccountItem[]
    ) => {
      for (const account of accounts) {
        if ("accounts" in account) {
          // Nested accounts struct
          processAccounts(account.accounts);
          continue;
        }

        const typeName = getDefinedTypeName(account);
        if (!typeName) continue;

        // Check if this type is compressible
        if (!isCompressibleType(typeName, idlTypes)) continue;

        const address = translateAddress(
          this._accounts[account.name] as Address
        );
        if (!address) {
          console.warn(
            `Compressible account ${account.name} not found in accounts`
          );
          continue;
        }

        const isCToken = isCTokenDataType(typeName, idlTypes);
        const variant = isCToken
          ? extractTokenVariant(account.name, typeName, idlTypes)
          : undefined;

        compressibleAccounts.push({
          name: account.name,
          address,
          typeName,
          isCToken,
          variant,
        });
      }
    };

    processAccounts(idlIx.accounts);

    if (compressibleAccounts.length === 0) {
      return [];
    }

    // Ensure Rpc instance (lightprotocol) is available
    let rpc: Rpc = (provider as any).rpc;
    if (!rpc) {
      rpc = createRpc(provider.connection);
      (provider as any).rpc = rpc;
    }

    // Batch fetch all compressible accounts in parallel

    const fetchPromises = compressibleAccounts.map(async (acc) => {
      try {
        const info = await getAccountInterface(rpc, acc.address);

        return {
          address: acc.address,
          info: {
            accountInfo: info.accountInfo,
            parsed: info.parsed,
            merkleContext: info.merkleContext,
          },
          accountType: acc.isCToken ? "cTokenData" : acc.typeName,
          tokenVariant: acc.variant,
        };
      } catch (err) {
        console.warn(`Failed to fetch compressible account ${acc.name}:`, err);
        return null;
      }
    });

    const results = await Promise.all(fetchPromises);
    return results.filter((r) => r !== null);
  }

  /**
   * Internal method to inject decompress instruction if needed.
   * Fully automatic based on IDL analysis.
   */
  private async _injectDecompressIfNeeded(): Promise<void> {
    if (!this._enableAutoDecompress) return;

    // Detect and fetch compressible accounts
    const accountInputs = await this._detectAndFetchCompressibleAccounts();

    if (accountInputs.length === 0) return;

    const programId = this._accountsResolver["_programId"];
    const provider = this._accountsResolver["_provider"] as Provider & {
      rpc?: Rpc;
    };
    // Prefer a cached Rpc on provider; otherwise construct one from the Connection endpoint
    let rpc: Rpc = provider.rpc!;
    if (!rpc) {
      rpc = createRpc(provider.connection);
      (provider as any).rpc = rpc;
    }

    // Build decompress params using the helper
    const params = await buildDecompressParams(programId, rpc, accountInputs);

    if (!params) return; // No compressed accounts

    // Use Anchor's own instruction builder to create decompressAccountsIdempotent
    // This instruction exists in the IDL (generated by the proc macro)
    const decompressIx = await this._ixFn["decompressAccountsIdempotent"](
      params.proofOption,
      params.compressedAccounts,
      params.systemAccountsOffset,
      {
        accounts: this._accounts,
        remainingAccounts: params.remainingAccounts,
      }
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
