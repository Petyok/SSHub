import { randomBytes, type TArg, type TRet } from '../utils.ts';
import { type CurvePoint, type CurvePointCons } from './curve.ts';
import { type H2CDSTOpts } from './hash-to-curve.ts';
import { type IField } from './modular.ts';
/** Cryptographically secure random byte generator. */
export type RNG = typeof randomBytes;
/** Serialized participant identifier. Identifiers are hex to make comparison easier. */
export type Identifier = string;
/** Serialized point commitment. */
export type Commitment = Uint8Array;
/** Serialized scalar coefficient. */
export type Coefficient = Uint8Array;
/** Serialized Schnorr signature. */
export type Signature = Uint8Array;
/** Threshold participant counts. */
export type Signers = {
    /** Minimum number of signers required to produce a signature. */
    min: number;
    /** Maximum number of participants in the key set. */
    max: number;
};
/** Serialized secret key bytes. */
export type SecretKey = Uint8Array;
/** Byte array alias used by FROST public packages. */
export type Bytes = Uint8Array;
type Point = Uint8Array;
/** Public DKG round-1 broadcast plus proof of knowledge. */
export type DKG_Round1 = {
    /** Sender identifier. */
    identifier: Identifier;
    /** VSS commitment points. */
    commitment: TRet<Commitment[]>;
    /** Signature proving knowledge of the sender's secret coefficient. */
    proofOfKnowledge: TRet<Signature>;
};
/** Public DKG round-2 recipient share package. */
export type DKG_Round2 = {
    /** Sender identifier. */
    identifier: Identifier;
    /** Signing share for one receiver. */
    signingShare: TRet<Bytes>;
};
/** Internal mutable DKG state package. */
export type DKG_Secret = {
    /** Local participant identifier as a scalar. */
    identifier: bigint;
    /** Local secret polynomial coefficients while DKG is in progress. */
    coefficients?: bigint[];
    /** Local VSS commitment points. */
    commitment: TRet<Point[]>;
    /** Threshold participant counts. */
    signers: Signers;
    /** Cached round2 packages from the first successful round2 call. */
    round2Cache?: Record<Identifier, DKG_Round2>;
    /** Canonical authenticated round1 transcript from the first successful round2 call. */
    round1Cache?: DKG_Round1[];
    /** Current DKG state-machine step. */
    step?: 1 | 2 | 3;
};
/** Shared public FROST package for one key set. */
export type FrostPublic = {
    /** Threshold participant counts. */
    signers: Signers;
    /** Serialized commitment points; `commitments[0]` is the group public key. */
    commitments: TRet<Bytes[]>;
    /** Map from participant identifier to serialized verifying-share point. */
    verifyingShares: TRet<Record<Identifier, Bytes>>;
};
/** Secret FROST share for one participant. */
export type FrostSecret = {
    /** Participant identifier. */
    identifier: Identifier;
    /** Serialized scalar signing share. */
    signingShare: TRet<Bytes>;
};
/** Combined public and secret FROST packages for one participant. */
export type Key = {
    /** Shared public package. */
    public: FrostPublic;
    /** Participant secret package. */
    secret: FrostSecret;
};
/** Trusted-dealer output containing public data and all participant shares. */
export type DealerShares = {
    /** Shared public package. */
    public: FrostPublic;
    /** Map from participant identifier to its secret share. */
    secretShares: Record<Identifier, FrostSecret>;
};
/** Private nonce scalars used once during signing. */
export type Nonces = {
    /** Serialized hiding nonce scalar. */
    hiding: TRet<Bytes>;
    /** Serialized binding nonce scalar. */
    binding: TRet<Bytes>;
};
/** Public nonce commitments broadcast for one signing attempt. */
export type NonceCommitments = {
    /** Participant identifier. */
    identifier: Identifier;
    /** Serialized hiding nonce point. */
    hiding: TRet<Bytes>;
    /** Serialized binding nonce point. */
    binding: TRet<Bytes>;
};
/** Generated nonce package containing private nonces and public commitments. */
export type GenNonce = {
    /** Private nonce scalars. */
    nonces: Nonces;
    /** Public nonce commitments. */
    commitments: NonceCommitments;
};
/** Point interface required by the generic FROST implementation. */
export interface FROSTPoint<T extends CurvePoint<any, T>> extends CurvePoint<any, T> {
    /**
     * Adds another point.
     * @param rhs - Point to add.
     * @returns Point sum.
     */
    add(rhs: T): T;
    /**
     * Multiplies by a scalar.
     * @param rhs - Scalar multiplier.
     * @returns Scalar multiplication result.
     */
    multiply(rhs: bigint): T;
    /**
     * Compares two points.
     * @param rhs - Point to compare.
     * @returns Whether points are equal.
     */
    equals(rhs: T): boolean;
    /**
     * Serializes a point.
     * @param compressed - Whether to use compressed encoding.
     * @returns Encoded point bytes.
     */
    toBytes(compressed?: boolean): Bytes;
    /**
     * Clears the point cofactor.
     * @returns Cofactor-cleared point.
     */
    clearCofactor(): T;
}
/** Point constructor surface required by FROST. */
export interface FROSTPointConstructor<T extends FROSTPoint<T>> extends CurvePointCons<T> {
    /**
     * Parses a point from bytes.
     * @param a - Encoded point bytes.
     * @returns Parsed point.
     */
    fromBytes(a: Bytes): T;
    /** Scalar field used by the point group. */
    Fn: IField<bigint>;
}
/** Construction options for a concrete FROST ciphersuite. */
export type FrostOpts<P extends FROSTPoint<P>> = {
    /** Ciphersuite name. */
    readonly name: string;
    /** Point constructor for the signing group. */
    readonly Point: FROSTPointConstructor<P>;
    /** Optional scalar-field override. */
    readonly Fn?: IField<bigint>;
    /**
     * Optional suite hook that adds checks after mandatory identity and subgroup validation.
     * @param p - Point to validate.
     */
    readonly validatePoint?: (p: P) => void;
    /**
     * Optional public-key parser. Its result is still subjected to the mandatory checks and
     * `validatePoint`; this hook only replaces byte decoding.
     * @param bytes - Encoded public key.
     * @returns Parsed public point.
     */
    readonly parsePublicKey?: (bytes: TArg<Uint8Array>) => P;
    /**
     * Hash function used by the suite.
     * @param msg - Message bytes to hash.
     * @returns Hash output bytes.
     */
    readonly hash: (msg: TArg<Uint8Array>) => TRet<Uint8Array>;
    /**
     * Custom scalar hash hook. Implementations MUST treat `msg` and `options` as read-only.
     * @param msg - Message bytes to hash.
     * @param options - Hash-to-curve options. See {@link H2CDSTOpts}.
     * @returns Scalar field element.
     */
    readonly hashToScalar?: (msg: TArg<Uint8Array>, options?: TArg<H2CDSTOpts>) => bigint;
    /**
     * Optional scalar adjustment hook.
     * @param n - Scalar to adjust.
     * @returns Adjusted scalar.
     */
    readonly adjustScalar?: (n: bigint) => bigint;
    /**
     * Optional point adjustment hook.
     * @param n - Point to adjust.
     * @returns Adjusted point.
     */
    readonly adjustPoint?: (n: P) => P;
    /**
     * Optional challenge override.
     * @param R - Group commitment point.
     * @param PK - Group public key point.
     * @param msg - Message bytes.
     * @returns Challenge scalar.
     */
    readonly challenge?: (R: P, PK: P, msg: TArg<Uint8Array>) => bigint;
    /**
     * Optional nonce-package adjustment hook.
     * @param R - Group commitment point for the current signing session.
     * @param nonces - Nonce package.
     * @returns Adjusted nonce package.
     */
    readonly adjustNonces?: (R: P, nonces: TArg<Nonces>) => TRet<Nonces>;
    /**
     * Optional secret-package adjustment hook.
     * @param secret - Secret package.
     * @param pub - Public package.
     * @returns Adjusted secret package.
     */
    readonly adjustSecret?: (secret: TArg<FrostSecret>, pub: TArg<FrostPublic>) => TRet<FrostSecret>;
    /**
     * Optional public-package adjustment hook.
     * @param pub - Public package.
     * @returns Adjusted public package.
     */
    readonly adjustPublic?: (pub: TArg<FrostPublic>) => TRet<FrostPublic>;
    /**
     * Optional group commitment-share adjustment hook.
     * @param GC - Group commitment.
     * @param GCShare - Participant commitment share.
     * @returns Adjusted group commitment share.
     */
    readonly adjustGroupCommitmentShare?: (GC: P, GCShare: P) => P;
    /** Optional transaction encoder / decoder adjustment. */
    readonly adjustTx?: {
        /**
         * Encode transaction bytes before signing.
         * @param tx - Transaction bytes.
         * @returns Encoded transaction bytes.
         */
        readonly encode: (tx: TArg<Uint8Array>) => TRet<Uint8Array>;
        /**
         * Decode transaction bytes after verification.
         * @param tx - Encoded transaction bytes.
         * @returns Decoded transaction bytes.
         */
        readonly decode: (tx: TArg<Uint8Array>) => TRet<Uint8Array>;
    };
    /**
     * Optional DKG output adjustment hook.
     * @param k - DKG key package.
     * @returns Adjusted DKG key package.
     */
    readonly adjustDKG?: (k: TArg<Key>) => TRet<Key>;
    /** Prefix for RFC 9591 H1. */
    readonly H1?: string;
    /** Prefix for RFC 9591 H2. */
    readonly H2?: string;
    /** Prefix for RFC 9591 H3. */
    readonly H3?: string;
    /** Prefix for RFC 9591 H4. */
    readonly H4?: string;
    /** Prefix for RFC 9591 H5. */
    readonly H5?: string;
    /** Prefix for DKG hashing. */
    readonly HDKG?: string;
    /** Prefix for identifier derivation. */
    readonly HID?: string;
};
/**
 * FROST: Threshold Protocol for Two‑Round Schnorr Signatures
 * from {@link https://datatracker.ietf.org/doc/rfc9591/ | RFC 9591}.
 */
export type FROST = {
    /** Methods to construct participant identifiers. */
    Identifier: {
        /**
         * Constructs an identifier from a numeric index.
         * @param n - A positive integer.
         * @returns A canonical serialized Identifier.
         */
        fromNumber(n: number): Identifier;
        /**
         * Derives an identifier deterministically from a string (e.g. an email).
         * @param s - Arbitrary string.
         * @returns A canonical serialized Identifier.
         */
        derive(s: string): Identifier;
    };
    /**
     * Distributed Key Generation (DKG) protocol interface.
     * RFC 9591 leaves DKG out of scope; Appendix C only specifies dealer/VSS key generation.
     * These helpers follow the split-round API used by frost-rs for interoperable testing.
     */
    DKG: {
        /**
         * Generates the first round of DKG.
         * @param id - Participant's identifier.
         * @param signers - Set of all participants (min/max threshold).
         * @param secret - Optional initial secret scalar.
         * @param rng - Optional RNG for nonce generation.
         * @returns Public broadcast and private DKG state. The returned `secret` package is mutable
         *   round state that will be consumed by `round2()` and `round3()`.
         */
        round1: (id: Identifier, signers: Signers, secret?: TArg<SecretKey>, rng?: RNG) => {
            public: DKG_Round1;
            secret: DKG_Secret;
        };
        /**
         * Executes DKG round 2 given public round1 data from others.
         * @param secret - Private DKG state from round1. This mutates `secret.step` in place.
         * @param others - Public round1 broadcasts from other participants.
         * @returns A map of round2 messages to be sent to others.
         */
        round2: (secret: TArg<DKG_Secret>, others: TArg<DKG_Round1[]>) => TRet<Record<string, DKG_Round2>>;
        /**
         * Finalizes key generation in round3 using received round1 + round2 messages.
         * @param secret - Private DKG state. This consumes the remaining local polynomial coefficients
         *   and transitions the package to its final post-round3 state.
         * @param round1 - Public round1 broadcasts from all participants.
         * @param round2 - Round2 messages received from others.
         * @returns Final secret/public key information for the participant.
         * `round1` must byte-compare equal to the remote packages authenticated in `round2()`.
         * Finalization consumes the stored authenticated transcript, not caller-owned arrays.
         */
        round3: (secret: TArg<DKG_Secret>, round1: TArg<DKG_Round1[]>, round2: TArg<DKG_Round2[]>) => TRet<Key>;
        /**
         * Best-effort erasure of internal secret state. Bigint/JIT copies may still survive outside the
         * local object even after cleanup.
         * @param secret - Private DKG state from round1.
         */
        clean(secret: TArg<DKG_Secret>): void;
    };
    /**
     * Trusted dealer mode: generates key shares from a central trusted authority.
     * Mirrors RFC 9591 Appendix C and returns one shared VSS commitment package
     * plus per-participant shares.
     * @param signers - Threshold parameters (min/max).
     * @param identifiers - Optional explicit participant list.
     * @param secret - Optional secret scalar.
     * @param rng - Optional RNG.
     * @returns One shared public package plus the participant secret-share packages.
     */
    trustedDealer(signers: Signers, identifiers?: Identifier[], secret?: TArg<SecretKey>, rng?: RNG): TRet<DealerShares>;
    /**
     * Validates the consistency of a secret share against the shared public commitments.
     * This is the RFC 9591 Appendix C.2 `vss_verify` check against the shared dealer/DKG commitment.
     * It does not relax RFC 9591 Section 3.1: public identity elements are still invalid even when
     * the scalar/share algebra would otherwise be self-consistent.
     * Throws if invalid.
     * @param secret - A FrostSecret containing identifier and signing share.
     * @param pub - Shared public package containing commitments.
     */
    validateSecret(secret: TArg<FrostSecret>, pub: TArg<FrostPublic>): void;
    /**
     * Produces nonces and public commitments used in signing.
     * RFC 9591 Section 5.1 `commit()`.
     * @param secret - Participant's secret share.
     * @param rng - Optional RNG.
     * @returns Nonce values and their public commitments.
     * Returned nonces are one-time-use and MUST NOT be reused across signing sessions.
     * This API does not mutate or zeroize caller-owned nonce objects.
     */
    commit(secret: TArg<FrostSecret>, rng?: RNG): TRet<GenNonce>;
    /**
     * Signs a message using the participant's secret and nonce.
     * @param secret - Participant's secret share.
     * @param pub - Shared public package containing commitments.
     * @param nonces - Participant's nonce pair.
     * @param commitmentList - Commitments from all signing participants.
     * @param msg - Message to be signed.
     * @returns Signature share as a byte array.
     * RFC 9591 Sections 4.1/5.1 require round-one commitments to be one-time-use, and
     * Section 5.2 signs with the nonce corresponding to that published commitment.
     * The caller MUST pass fresh nonces from `commit()`. On successful signing, this helper
     * consumes the caller-owned nonce object by zeroing both nonce byte arrays in place.
     * Later calls reject an all-zero nonce package, so same-object reuse fails closed and an
     * accidentally generated zero nonce package is not silently used for signing.
     */
    signShare(secret: TArg<FrostSecret>, pub: TArg<FrostPublic>, nonces: TArg<Nonces>, commitmentList: TArg<NonceCommitments[]>, msg: TArg<Uint8Array>): TRet<Uint8Array>;
    /**
     * Verifies a signature share against public commitments.
     * Matches the coordinator-side individual-share verification from RFC 9591 Section 5.4.
     * @param pub - Group public key information.
     * @param commitmentList - Commitments from all signing participants.
     * @param msg - Message being signed.
     * @param identifier - Identifier of the signer whose share is being verified.
     * @param sigShare - Signature share to verify.
     * @returns True if valid, false otherwise.
     */
    verifyShare(pub: TArg<FrostPublic>, commitmentList: TArg<NonceCommitments[]>, msg: TArg<Uint8Array>, identifier: Identifier, sigShare: TArg<Uint8Array>): boolean;
    /**
     * Aggregates signature shares into a full signature.
     * RFC 9591 Section 5.3 `aggregate()`.
     * @param pub - Group public key.
     * @param commitmentList - Nonce commitments from all signers.
     * @param msg - Message to sign.
     * @param sigShares - Map from identifier to their signature share.
     * @returns Final aggregated signature.
     */
    aggregate(pub: TArg<FrostPublic>, commitmentList: TArg<NonceCommitments[]>, msg: TArg<Uint8Array>, sigShares: TArg<Record<Identifier, Uint8Array>>): TRet<Uint8Array>;
    /**
     * Signs a message using a raw secret key (e.g. from combineSecret).
     * @param msg - Message to sign.
     * @param secretKey - Group secret key as bytes.
     * @returns Signature bytes.
     */
    sign(msg: TArg<Uint8Array>, secretKey: TArg<Uint8Array>): TRet<Uint8Array>;
    /**
     * Verifies a full signature against the group public key.
     * @param sig - Signature bytes.
     * @param msg - Message that was signed.
     * @param publicKey - Group public key.
     * @returns True if valid, false otherwise.
     */
    verify(sig: TArg<Signature>, msg: TArg<Uint8Array>, publicKey: TArg<Uint8Array>): boolean;
    /**
     * Combines multiple secret shares into a single secret key (e.g. for recovery).
     * @param shares - Set of FrostSecret shares.
     * @param signers - Threshold parameters.
     * @returns Group secret key as bytes.
     */
    combineSecret(shares: TArg<FrostSecret[]>, signers: Signers): TRet<Uint8Array>;
    /** Low-level helper utilities (field arithmetic and polynomial tools). */
    utils: {
        /**
         * Finite field used for scalars.
         */
        Fn: IField<bigint>;
        /**
         * Generates a random scalar (private key).
         * @param rng - Optional RNG source.
         * @returns Scalar as 32-byte Uint8Array.
         */
        randomScalar: (rng?: RNG) => TRet<Uint8Array>;
        /**
         * Generates a secret-sharing polynomial and its public commitments.
         * @param signers - Threshold parameters.
         * @param secret - Optional initial secret scalar.
         * @param coeffs - Optional manual coefficients.
         * @param rng - Optional RNG.
         * @returns Polynomial coefficients, commitments, and secret value.
         */
        generateSecretPolynomial: (signers: Signers, secret?: TArg<Uint8Array>, coeffs?: bigint[], rng?: RNG) => {
            coefficients: bigint[];
            commitment: TRet<Point[]>;
            secret: bigint;
        };
    };
};
/**
 * Builds a FROST ciphersuite API from concrete curve and hash hooks.
 * @param opts - Ciphersuite construction options. See {@link FrostOpts}.
 * @returns FROST API bound to the supplied ciphersuite.
 * @example
 * Create a suite from a curve-specific option object.
 * ```ts
 * import { createFROST } from '@noble/curves/abstract/frost.js';
 * import { ed25519 } from '@noble/curves/ed25519.js';
 * import { sha512 } from '@noble/hashes/sha2.js';
 * const frost = createFROST({
 *   name: 'FROST-ED25519-SHA512-v1',
 *   Point: ed25519.Point,
 *   hash: sha512,
 * });
 * ```
 */
export declare function createFROST<P extends FROSTPoint<P>>(opts: FrostOpts<P>): TRet<FROST>;
export {};
