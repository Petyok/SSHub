import { type BlsCurvePairWithSignatures } from './abstract/bls.ts';
import { type IField } from './abstract/modular.ts';
import { type TRet } from './utils.ts';
/**
 * bls12-381 Fr (Fn) field.
 * `fromBytes()` reduces modulo `q` instead of rejecting non-canonical encodings.
 */
export declare const bls12_381_Fr: TRet<IField<bigint>>;
/**
 * bls12-381 pairing-friendly curve construction.
 * Provides both longSignatures and shortSignatures.
 * @example
 * bls12-381 pairing-friendly curve construction.
 *
 * ```ts
 * const bls = bls12_381.longSignatures;
 * const { secretKey, publicKey } = bls.keygen();
 * const msg = bls.hash(new TextEncoder().encode('hello noble'));
 * const sig = bls.sign(msg, secretKey);
 * const isValid = bls.verify(sig, msg, publicKey);
 * ```
 */
export declare const bls12_381: BlsCurvePairWithSignatures;
