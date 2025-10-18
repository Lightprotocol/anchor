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
   * System accounts (feePayer, config, etc.) are REQUIRED.
   * Seed accounts (lpMint, ammConfig, etc.) are OPTIONAL - only provide them
   * if their dependent compressible accounts are being decompressed.
   *
   * Automatically infers account types and variants from IDL, fetches compression state,
   * and injects decompress instruction if needed.
   *
   * @param accounts - Accounts for decompressAccountsIdempotent instruction.
   *                   System accounts required, seed accounts optional.
   * @returns this builder for chaining
   *
   * @example
   * ```ts
   * await program.methods
   *   .swap(amount)
   *   .decompressIfNeeded({
   *     // System accounts (required)
   *     feePayer: owner.publicKey,
   *     config: compressionConfig,
   *     rentPayer: owner.publicKey,
   *     ctokenRentSponsor: CTOKEN_RENT_SPONSOR,
   *     ctokenProgram: CompressedTokenProgram.programId,
   *     ctokenCpiAuthority: CompressedTokenProgram.deriveCpiAuthorityPda,
   *     ctokenConfig,
   *     // Seed accounts (optional - only provide what's needed)
   *     ammConfig,      // Only if poolState is compressed
   *     token0Mint,     // Only if token0Vault is compressed
   *     token1Mint,     // Only if token1Vault is compressed
   *     // lpMint: omitted because lpVault not being decompressed
   *     // Compressible accounts
   *     poolState,
   *     token0Vault,
   *     token1Vault,
   *   })
   *   .accounts({ ... })
   *   .rpc();
   * ```
   */
  public decompressIfNeeded(
    accounts: Partial<Accounts<D>> & { [name: string]: Address }
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
   */
  private async _fetchCompressibleAccounts(
    accountsToCheck: Array<{ name: string; address: PublicKey }>
  ): Promise<AccountInput[]> {
    if (accountsToCheck.length === 0) return [];

    const programId = this._programId;
    const rpc = createRpc();

    const defaultAddressTreeInfo = getDefaultAddressTreeInfo();

    const accountInputs: AccountInput[] = [];

    // Check each provided account to see if it's actually compressed
    for (const { name, address } of accountsToCheck) {
      try {
        // Try fetching as CPDA first
        // @ts-ignore
        const cpdaResult = await rpc.getAccountInfoInterface(
          address,
          programId,
          //@ts-ignore
          defaultAddressTreeInfo
        );

        if (cpdaResult && cpdaResult.merkleContext && cpdaResult.isCompressed) {
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
        } else {
          // Try as compressed token via getAccountInterface as fallback
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
            // Not a compressed account
          }
        }
      } catch {
        // Account doesn't exist or can't be fetched, skip
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
   * Helper to get account from main instruction if it exists with the same name
   */
  private _getFromMainInstruction(accountName: string): PublicKey | null {
    const programId = this._programId;
    if (this._accounts && accountName in this._accounts) {
      try {
        const pubkey = translateAddress(this._accounts[accountName] as Address);
        if (!pubkey.equals(programId)) {
          console.log(
            `[decompressIfNeeded] Using '${accountName}' from main instruction:`,
            pubkey.toBase58()
          );
          return pubkey;
        }
      } catch {
        // Invalid address, ignore
      }
    }
    return null;
  }

  /**
   * Extract required seed accounts based on what's actually being decompressed
   */
  private _extractRequiredSeeds(accountInputs: AccountInput[]): Set<string> {
    const requiredSeeds = new Set<string>();

    // Helper to extract seed account names from IDL PDA definition
    const extractSeedsFromIdl = (accountName: string): string[] => {
      if (!this._idl) return [];

      const findAccount = (accounts: readonly any[], name: string): any => {
        for (const acc of accounts) {
          if ("accounts" in acc) {
            const found = findAccount(acc.accounts, name);
            if (found) return found;
          } else if (acc.name === accountName) {
            return acc;
          }
        }
        return null;
      };

      // Search all instructions for PDA definition
      for (const ix of this._idl.instructions) {
        const accountDef = findAccount(ix.accounts, accountName);
        if (accountDef && accountDef.pda && accountDef.pda.seeds) {
          const seeds: string[] = [];
          for (const seed of accountDef.pda.seeds) {
            if (seed.kind === "account" && seed.path) {
              const baseName = seed.path.split(".")[0].split("(")[0];
              seeds.push(baseName);
            }
          }
          return seeds;
        }
      }
      return [];
    };

    // Analyze each account being decompressed
    for (const input of accountInputs) {
      const accountName = input.tokenVariant || (input.accountType as string);

      // Extract seeds from IDL for this account
      const seeds = extractSeedsFromIdl(accountName);
      for (const seed of seeds) {
        requiredSeeds.add(seed);
        console.log(
          `[decompressIfNeeded] Account '${accountName}' requires seed: '${seed}' (from IDL)`
        );
      }
    }

    return requiredSeeds;
  }

  /**
   * Get compressible account names from main instruction's IDL definition
   */
  private _getMainInstructionCompressibleAccounts(): Set<string> {
    const compressible = new Set<string>();
    if (!this._idl) return compressible;

    // Find the main instruction in IDL
    const mainIx = this._idl.instructions.find(
      (ix: any) => ix.name === (this._idlIx as any).name
    );
    if (!mainIx) return compressible;

    // Recursively scan all accounts
    const visitAccounts = (items: readonly any[]) => {
      for (const item of items) {
        if ("accounts" in item) {
          visitAccounts(item.accounts);
          continue;
        }
        // Check the compressible field
        if (item.compressible === true) {
          compressible.add(item.name);
        }
      }
    };

    visitAccounts(mainIx.accounts);
    return compressible;
  }

  /**
   * Auto-pull accounts from main instruction for matching names
   */
  private _autoPullFromMainInstruction(): void {
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

    // Try to pull each missing account from main instruction
    for (const accountName in this._decompressAccounts) {
      if (isAccountMissing(accountName)) {
        const fromMain = this._getFromMainInstruction(accountName);
        if (fromMain) {
          console.log(
            `[decompressIfNeeded] Auto-pulling '${accountName}' from main instruction:`,
            fromMain.toBase58()
          );
          this._decompressAccounts[accountName] = fromMain;
        }
      }
    }
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

    // Collect potentially compressible accounts
    // Only check accounts that were explicitly provided (not set to programId placeholder)
    const accountsToCheck: Array<{ name: string; address: PublicKey }> = [];

    for (const [name, addr] of Object.entries(this._decompressAccounts)) {
      if (SKIP_ACCOUNTS.has(name)) continue;

      try {
        const pubkey = translateAddress(addr as Address);

        // Skip accounts set to programId - these are explicitly marked as "None/not needed"
        if (pubkey.equals(this._programId)) {
          console.log(
            `[decompressIfNeeded] Skipping account '${name}' (set to programId = None)`
          );
          continue;
        }

        // All provided non-system, non-placeholder accounts should be checked
        accountsToCheck.push({ name, address: pubkey });
      } catch {
        console.warn(
          `[decompressIfNeeded] Invalid address for ${name}, skipping`
        );
      }
    }

    // Validate: All compressible accounts used by main instruction must be provided
    const mainIxCompressible = this._getMainInstructionCompressibleAccounts();
    const providedCompressible = new Set(accountsToCheck.map((a) => a.name));
    const missing: string[] = [];

    for (const accountName of Array.from(mainIxCompressible)) {
      const inMain = this._getFromMainInstruction(accountName);
      if (inMain && !providedCompressible.has(accountName)) {
        // Main instruction uses this compressible account, but not checked for decompression
        missing.push(accountName);
      }
    }

    if (missing.length > 0) {
      throw new Error(
        `[decompressIfNeeded] Missing compressible accounts used by main instruction: ${missing.join(
          ", "
        )}.\n` +
          `These accounts are marked as compressible in the IDL and must be provided to decompressIfNeeded().\n` +
          `Add them: .decompressIfNeeded({ ..., ${missing
            .map((n) => `${n}: <address>`)
            .join(", ")} })`
      );
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

    // Auto-resolve constant and default accounts before validation
    this._autoResolveDecompressAccounts();

    // Determine which seed accounts are required based on what's being decompressed
    const requiredSeeds = this._extractRequiredSeeds(accountInputs);
    console.log(
      `[decompressIfNeeded] Required seeds for decompression:`,
      Array.from(requiredSeeds)
    );

    // Auto-pull required seeds from main instruction
    for (const seedName of Array.from(requiredSeeds)) {
      if (
        !(seedName in this._decompressAccounts) ||
        translateAddress(this._decompressAccounts[seedName] as Address).equals(
          programId
        )
      ) {
        const fromMain = this._getFromMainInstruction(seedName);
        if (fromMain) {
          console.log(
            `[decompressIfNeeded] Auto-pulling required seed '${seedName}' from main instruction:`,
            fromMain.toBase58()
          );
          this._decompressAccounts[seedName] = fromMain;
        } else {
          throw new Error(
            `[decompressIfNeeded] Required seed account '${seedName}' is missing. ` +
              `It's needed for decompressing but not found in main instruction. ` +
              `Please provide it explicitly in decompressIfNeeded() call.`
          );
        }
      }
    }

    // Fill in any missing accounts with program ID (represents "None" for optional accounts)
    // Only fill accounts that are NOT required seeds
    const completeAccounts = { ...this._decompressAccounts };
    const decompressIxAccounts = decompressInstruction.accounts || [];

    for (const acc of decompressIxAccounts) {
      const accountName = typeof acc === "string" ? acc : acc.name;
      if (!(accountName in completeAccounts)) {
        if (requiredSeeds.has(accountName)) {
          throw new Error(
            `[decompressIfNeeded] Required seed account '${accountName}' is missing.`
          );
        }
        console.log(
          `[decompressIfNeeded] Auto-filling optional account '${accountName}' with program ID (not needed for current decompression)`
        );
        completeAccounts[accountName] = programId;
      }
    }

    console.log(
      `[decompressIfNeeded] Building instruction with accounts:`,
      Object.keys(completeAccounts)
    );

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
