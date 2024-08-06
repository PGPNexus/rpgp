use aes_gcm::aead::rand_core::CryptoRng;
use chrono::SubsecRound;
use rand::Rng;
use smallvec::SmallVec;

use crate::composed::SignedKeyDetails;
use crate::crypto::aead::AeadAlgorithm;
use crate::crypto::hash::HashAlgorithm;
use crate::crypto::sym::SymmetricKeyAlgorithm;
use crate::errors::Result;
use crate::packet::{
    KeyFlags, PacketTrait, SignatureConfigBuilder, SignatureType, Subpacket, SubpacketData,
    UserAttribute, UserId,
};
use crate::types::{CompressionAlgorithm, RevocationKey, SecretKeyTrait};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct KeyDetails {
    primary_user_id: UserId,
    user_ids: Vec<UserId>,
    user_attributes: Vec<UserAttribute>,
    keyflags: KeyFlags,
    preferred_symmetric_algorithms: SmallVec<[SymmetricKeyAlgorithm; 8]>,
    preferred_hash_algorithms: SmallVec<[HashAlgorithm; 8]>,
    preferred_compression_algorithms: SmallVec<[CompressionAlgorithm; 8]>,
    preferred_aead_algorithms: SmallVec<[(SymmetricKeyAlgorithm, AeadAlgorithm); 4]>,
    revocation_key: Option<RevocationKey>,
}

impl KeyDetails {
    #[allow(clippy::too_many_arguments)] // FIXME
    pub fn new(
        primary_user_id: UserId,
        user_ids: Vec<UserId>,
        user_attributes: Vec<UserAttribute>,
        keyflags: KeyFlags,
        preferred_symmetric_algorithms: SmallVec<[SymmetricKeyAlgorithm; 8]>,
        preferred_hash_algorithms: SmallVec<[HashAlgorithm; 8]>,
        preferred_compression_algorithms: SmallVec<[CompressionAlgorithm; 8]>,
        preferred_aead_algorithms: SmallVec<[(SymmetricKeyAlgorithm, AeadAlgorithm); 4]>,
        revocation_key: Option<RevocationKey>,
    ) -> Self {
        KeyDetails {
            primary_user_id,
            user_ids,
            user_attributes,
            keyflags,
            preferred_symmetric_algorithms,
            preferred_hash_algorithms,
            preferred_compression_algorithms,
            preferred_aead_algorithms,
            revocation_key,
        }
    }

    pub fn sign<R, F>(
        self,
        mut rng: &mut R,
        key: &impl SecretKeyTrait,
        key_pw: F,
    ) -> Result<SignedKeyDetails>
    where
        R: CryptoRng + Rng,
        F: (FnOnce() -> String) + Clone,
    {
        let sig_version = key.version().try_into()?;

        let keyflags: SmallVec<[u8; 1]> = self.keyflags.into();
        let preferred_symmetric_algorithms = self.preferred_symmetric_algorithms;
        let preferred_hash_algorithms = self.preferred_hash_algorithms;
        let preferred_compression_algorithms = self.preferred_compression_algorithms;
        let preferred_aead_algorithms = self.preferred_aead_algorithms;
        let revocation_key = self.revocation_key;

        let mut users = vec![];

        // primary user id
        {
            let id = self.primary_user_id;
            let mut hashed_subpackets = vec![
                Subpacket::regular(SubpacketData::IsPrimary(true)),
                Subpacket::regular(SubpacketData::SignatureCreationTime(
                    chrono::Utc::now().trunc_subsecs(0),
                )),
                Subpacket::regular(SubpacketData::KeyFlags(keyflags.clone())),
                Subpacket::regular(SubpacketData::PreferredSymmetricAlgorithms(
                    preferred_symmetric_algorithms.clone(),
                )),
                Subpacket::regular(SubpacketData::PreferredHashAlgorithms(
                    preferred_hash_algorithms.clone(),
                )),
                Subpacket::regular(SubpacketData::PreferredCompressionAlgorithms(
                    preferred_compression_algorithms.clone(),
                )),
                Subpacket::regular(SubpacketData::PreferredAeadAlgorithms(
                    preferred_aead_algorithms.clone(),
                )),
                Subpacket::regular(SubpacketData::IssuerFingerprint(
                    key.version(),
                    key.fingerprint(),
                )),
            ];
            if let Some(rkey) = revocation_key {
                hashed_subpackets.push(Subpacket::regular(SubpacketData::RevocationKey(rkey)));
            }

            let hash_alg = key.hash_alg();

            let salt = crate::types::salt_for(rng, sig_version, hash_alg);

            let config = SignatureConfigBuilder::default()
                .version(sig_version)
                .typ(SignatureType::CertGeneric)
                .pub_alg(key.algorithm())
                .hash_alg(hash_alg)
                .hashed_subpackets(hashed_subpackets)
                .unhashed_subpackets(vec![Subpacket::regular(SubpacketData::Issuer(
                    key.key_id(),
                ))])
                .salt(salt)
                .build()?;

            let sig = config.sign_certification(key, key_pw.clone(), id.tag(), &id)?;

            users.push(id.into_signed(sig));
        }

        // other user ids

        users.extend(
            self.user_ids
                .into_iter()
                .map(|id| {
                    let hash_alg = key.hash_alg();

                    let salt = crate::types::salt_for(rng, sig_version, hash_alg);

                    let config = SignatureConfigBuilder::default()
                        .version(sig_version)
                        .typ(SignatureType::CertGeneric)
                        .pub_alg(key.algorithm())
                        .hash_alg(hash_alg)
                        .hashed_subpackets(vec![
                            Subpacket::regular(SubpacketData::SignatureCreationTime(
                                chrono::Utc::now().trunc_subsecs(0),
                            )),
                            Subpacket::regular(SubpacketData::KeyFlags(keyflags.clone())),
                            Subpacket::regular(SubpacketData::PreferredSymmetricAlgorithms(
                                preferred_symmetric_algorithms.clone(),
                            )),
                            Subpacket::regular(SubpacketData::PreferredHashAlgorithms(
                                preferred_hash_algorithms.clone(),
                            )),
                            Subpacket::regular(SubpacketData::PreferredCompressionAlgorithms(
                                preferred_compression_algorithms.clone(),
                            )),
                            Subpacket::regular(SubpacketData::PreferredAeadAlgorithms(
                                preferred_aead_algorithms.clone(),
                            )),
                            Subpacket::regular(SubpacketData::IssuerFingerprint(
                                key.version(),
                                key.fingerprint(),
                            )),
                        ])
                        .unhashed_subpackets(vec![Subpacket::regular(SubpacketData::Issuer(
                            key.key_id(),
                        ))])
                        .salt(salt)
                        .build()?;

                    let sig = config.sign_certification(key, key_pw.clone(), id.tag(), &id)?;

                    Ok(id.into_signed(sig))
                })
                .collect::<Result<Vec<_>>>()?,
        );

        let user_attributes = self
            .user_attributes
            .into_iter()
            .map(|u| u.sign(&mut rng, key, key_pw.clone()))
            .collect::<Result<Vec<_>>>()?;

        Ok(SignedKeyDetails {
            revocation_signatures: Default::default(),
            direct_signatures: Default::default(),
            users,
            user_attributes,
        })
    }
}
