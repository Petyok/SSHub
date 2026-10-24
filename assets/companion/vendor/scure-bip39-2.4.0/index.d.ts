import { type TArg, type TRet } from '@noble/hashes/utils.js';
/**
 * Generate x random words. Uses Cryptographically-Secure Random Number Generator.
 * @param wordlist - Imported wordlist for a specific language.
 * @param strength - Mnemonic strength, from 128 to 256 bits.
 * @returns 12-24 word mnemonic phrase.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Generate a new English mnemonic.
 * ```ts
 * import { generateMnemonic } from '@scure/bip39';
 * import { wordlist } from '@scure/bip39/wordlists/english.js';
 * const mnemonic = generateMnemonic(wordlist, 128);
 * // 'legal winner thank year wave sausage worth useful legal winner thank yellow'
 * ```
 */
export declare function generateMnemonic(wordlist: string[], strength?: number): string;
/**
 * Reversible: Converts mnemonic string to raw entropy in form of byte array.
 * @param mnemonic - 12-24 words.
 * @param wordlist - Imported wordlist for a specific language.
 * @returns Raw entropy bytes.
 * @throws If the mnemonic shape or checksum is invalid. {@link Error}
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Decode a mnemonic back into its original entropy bytes.
 * ```ts
 * import { mnemonicToEntropy } from '@scure/bip39';
 * import { wordlist } from '@scure/bip39/wordlists/english.js';
 * const mnem = 'legal winner thank year wave sausage worth useful legal winner thank yellow';
 * const entropy = mnemonicToEntropy(mnem, wordlist);
 * // Produces the original 16-byte entropy payload.
 * new Uint8Array([
 *   0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f,
 *   0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f
 * ])
 * ```
 */
export declare function mnemonicToEntropy(mnemonic: string, wordlist: string[]): TRet<Uint8Array>;
/**
 * Reversible: Converts raw entropy in form of byte array to mnemonic string.
 * @param entropy - Byte array.
 * @param wordlist - Imported wordlist for a specific language.
 * @returns 12-24 words.
 * @throws On wrong argument types. {@link TypeError}
 * @throws On wrong argument ranges or values. {@link RangeError}
 * @example
 * Convert raw entropy into an English mnemonic.
 * ```ts
 * import { entropyToMnemonic } from '@scure/bip39';
 * import { wordlist } from '@scure/bip39/wordlists/english.js';
 * const ent = new Uint8Array([
 *   0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f,
 *   0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f
 * ]);
 * const mnemonic = entropyToMnemonic(ent, wordlist);
 * // 'legal winner thank year wave sausage worth useful legal winner thank yellow'
 * ```
 */
export declare function entropyToMnemonic(entropy: TArg<Uint8Array>, wordlist: string[]): string;
/**
 * Validates mnemonic for being 12-24 words contained in `wordlist`.
 * @param mnemonic - 12-24 words.
 * @param wordlist - Imported wordlist for a specific language.
 * @returns `true` when mnemonic checksum and words are valid.
 * @example
 * Validate one English mnemonic.
 * ```ts
 * import { validateMnemonic } from '@scure/bip39';
 * import { wordlist } from '@scure/bip39/wordlists/english.js';
 * const ok = validateMnemonic(
 *   'legal winner thank year wave sausage worth useful legal winner thank yellow',
 *   wordlist
 * );
 * // => true
 * ```
 */
export declare function validateMnemonic(mnemonic: string, wordlist: string[]): boolean;
/**
 * Irreversible: Uses KDF to derive 64 bytes of key data from mnemonic + optional password.
 * @param mnemonic - 12-24 words.
 * @param passphrase - String that will additionally protect the key.
 * @returns 64 bytes of key data.
 * @throws If the mnemonic shape is invalid. {@link Error}
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Derive a seed from a mnemonic with the async PBKDF2 helper.
 * ```ts
 * const mnem = 'legal winner thank year wave sausage worth useful legal winner thank yellow';
 * const seed = await mnemonicToSeed(mnem, 'password');
 * // => new Uint8Array([...64 bytes])
 * ```
 */
export declare function mnemonicToSeed(mnemonic: string, passphrase?: string): Promise<TRet<Uint8Array>>;
/**
 * Irreversible: Uses KDF to derive 64 bytes of key data from mnemonic + optional password.
 * @param mnemonic - 12-24 words.
 * @param passphrase - String that will additionally protect the key.
 * @returns 64 bytes of key data.
 * @throws If the mnemonic shape is invalid. {@link Error}
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Derive a seed from a mnemonic with the sync PBKDF2 helper.
 * ```ts
 * const mnem = 'legal winner thank year wave sausage worth useful legal winner thank yellow';
 * const seed = mnemonicToSeedSync(mnem, 'password');
 * // => new Uint8Array([...64 bytes])
 * ```
 */
export declare function mnemonicToSeedSync(mnemonic: string, passphrase?: string): TRet<Uint8Array>;
/**
 * Uses native, built-in functionality, provided by globalThis.crypto.
 * Irreversible: Uses KDF to derive 64 bytes of key data from mnemonic + optional password.
 * @param mnemonic - 12-24 words.
 * @param passphrase - String that will additionally protect the key.
 * @returns 64 bytes of key data.
 * @throws If the mnemonic shape is invalid. {@link Error}
 * @throws On wrong argument types. {@link TypeError}
 * @example
 * Derive a seed with the native WebCrypto PBKDF2 helper.
 * ```ts
 * const mnem = 'legal winner thank year wave sausage worth useful legal winner thank yellow';
 * const seed = await mnemonicToSeedWebcrypto(mnem, 'password');
 * // => new Uint8Array([...64 bytes])
 * ```
 */
export declare function mnemonicToSeedWebcrypto(mnemonic: string, passphrase?: string): Promise<TRet<Uint8Array>>;
