/* Copyright (c) Fortanix, Inc.
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use yasna::{ASN1Error, ASN1ErrorKind, ASN1Result, BERReader, DERWriter, BERDecodable, Tag};
use num_integer::Integer;
use num_bigint::BigUint;
use std::borrow::Cow;
use bit_vec::BitVec;
use oid;

use DerWrite;
use types::*;

pub type RsaPkcs15TbsCertificate<'a> = TbsCertificate<BigUint, RsaPkcs15<Sha256>, DerSequence<'a>>;
pub type RsaPkcs15Certificate<'a> = Certificate<RsaPkcs15TbsCertificate<'a>, RsaPkcs15<Sha256>, BitVec>;

pub type GenericTbsCertificate = TbsCertificate<BigUint, DerSequence<'static>, DerSequence<'static>>;

pub type GenericCertificate = Certificate<GenericTbsCertificate, DerSequence<'static>, BitVec>;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Certificate<T, A: SignatureAlgorithm, S> {
    pub tbscert: T,
    pub sigalg: A,
    pub sig: S,
}

impl<T: DerWrite, A: SignatureAlgorithm + DerWrite, S: DerWrite> DerWrite for Certificate<T, A, S> {
    fn write(&self, writer: DERWriter) {
        writer.write_sequence(|writer| {
            self.tbscert.write(writer.next());
            self.sigalg.write(writer.next());
            self.sig.write(writer.next());
        });
    }
}

impl<T: BERDecodable, A: SignatureAlgorithm + BERDecodable, S: BERDecodable> BERDecodable for Certificate<T, A, S> {
    fn decode_ber<'a, 'b>(reader: BERReader<'a, 'b>) -> ASN1Result<Self> {
        reader.read_sequence(|r| {
            let tbscert = T::decode_ber(r.next())?;
            let sigalg = A::decode_ber(r.next())?;
            let sig = S::decode_ber(r.next())?;

            Ok(Certificate { tbscert, sigalg, sig })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TbsCertificate<S: Integer, A: SignatureAlgorithm, K> {
    pub version: u8,
    pub serial: S,
    pub sigalg: A,
    pub issuer: Name,
    pub validity_notbefore: DateTime,
    pub validity_notafter: DateTime,
    pub subject: Name,
    pub spki: K,
    pub extensions: Vec<Extension>,
}

impl<S: Integer, A: SignatureAlgorithm, K> TbsCertificate<S, A, K> {
    /// Find the extension indicated by `oid`. If exactly one instance
    /// of the extension is present, returns it. Otherwise, returns `None`.
    pub fn get_extension(&self, oid: &ObjectIdentifier) -> Option<Extension> {
        let mut iter = self.extensions.iter().filter(|e| e.oid == *oid);

        match (iter.next(), iter.next()) {
            (Some(ext), None) => Some(ext.to_owned()),
            _ => None,
        }
    }
}

pub const TBS_CERTIFICATE_V1: u8 = 0;
pub const TBS_CERTIFICATE_V2: u8 = 1;
pub const TBS_CERTIFICATE_V3: u8 = 2;

impl<S: DerWrite + Integer, A: DerWrite + SignatureAlgorithm, K: DerWrite> DerWrite
    for TbsCertificate<S, A, K> {
    fn write(&self, writer: DERWriter) {
        writer.write_sequence(|writer| {
            if self.version != TBS_CERTIFICATE_V1 { // default value
                writer.next().write_tagged(Tag::context(0), |w| self.version.write(w));
            }
            self.serial.write(writer.next());
            self.sigalg.write(writer.next());
            self.issuer.write(writer.next());
            writer.next().write_sequence(|writer| {
                self.validity_notbefore.write(writer.next());
                self.validity_notafter.write(writer.next());
            });
            self.subject.write(writer.next());
            self.spki.write(writer.next());
            if !self.extensions.is_empty() {
                writer.next().write_tagged(Tag::context(3), |w| {
                    w.write_sequence(|writer| {
                        for ext in &self.extensions {
                            ext.write(writer.next())
                        }
                    })
                });
            }
        });
    }
}

impl<S: BERDecodable + Integer, A: BERDecodable + SignatureAlgorithm, K: BERDecodable> BERDecodable for TbsCertificate<S, A, K> {
    fn decode_ber<'a, 'b>(reader: BERReader<'a, 'b>) -> ASN1Result<Self> {
        reader.read_sequence(|r| {
            let version = r.read_optional(|r| r.read_tagged(Tag::context(0), |r| r.read_u8()))?.unwrap_or(0);
            match version {
                TBS_CERTIFICATE_V1 | TBS_CERTIFICATE_V2 | TBS_CERTIFICATE_V3 => { /* known version */ }
                _ => { return Err(ASN1Error::new(ASN1ErrorKind::Invalid)); }
            };
            let serial = S::decode_ber(r.next())?;
            let sigalg = A::decode_ber(r.next())?;
            let issuer = Name::decode_ber(r.next())?;
            let (validity_notbefore,
                 validity_notafter) = r.next().read_sequence(|r| {
                Ok((DateTime::decode_ber(r.next())?,
                    DateTime::decode_ber(r.next())?))
            })?;
            let subject = Name::decode_ber(r.next())?;
            let spki = K::decode_ber(r.next())?;
            let extensions = r.read_optional(|r| {
                if version != TBS_CERTIFICATE_V3 {
                    return Err(ASN1Error::new(ASN1ErrorKind::Invalid));
                }
                r.read_tagged(Tag::context(3), |r| {
                    r.read_sequence(|r| {
                        let mut extensions = Vec::<Extension>::new();

                        loop {
                            let res = r.read_optional(|r| {
                                Extension::decode_ber(r)
                            });
                            match res {
                                Ok(Some(ext)) => extensions.push(ext),
                                Ok(None) => break,
                                Err(e) => return Err(e),
                            }
                        }

                        Ok(extensions)
                    })
                })
            })?.unwrap_or(vec![]);

            Ok(TbsCertificate { version, serial, sigalg, issuer, validity_notbefore, validity_notafter,
                                subject, spki, extensions })
        })
    }
}

/// BasicConstraints type as defined in RFC 5280.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct BasicConstraints {
    ca: bool,
    path_len_constraint: Option<u64>,
}

impl BasicConstraints {
    pub fn ca(path_len_constraint: Option<u64>) -> Self {
        BasicConstraints { ca: true, path_len_constraint }
    }

    pub fn no_ca() -> Self {
        BasicConstraints { ca: false, path_len_constraint: None }
    }

    /// Returns the value of the `cA` field, as defined in RFC 5280.
    pub fn is_ca(&self) -> bool {
        self.ca
    }

    /// Returns the value of the `pathLenConstraint` field, as defined in RFC 5280.
    ///
    /// Note that the RFC states that the `pathLenConstraint` field is meaningful only if the
    /// `cA` boolean is asserted. It is up to the users of this method to enforce that statement.
    pub fn path_len_constraint(&self) -> Option<u64> {
        self.path_len_constraint
    }
}

impl HasOid for BasicConstraints {
    fn oid() -> &'static ObjectIdentifier {
        &oid::basicConstraints
    }
}

impl DerWrite for BasicConstraints {
    fn write(&self, writer: DERWriter) {
        writer.write_sequence(|writer| {
            if self.ca { // do not write field if `ca` is equal to default value `false`
                writer.next().write_bool(true);
            }
            if let Some(path_len) = self.path_len_constraint {
                writer.next().write_u64(path_len);
            }
        });
    }
}

impl BERDecodable for BasicConstraints {
    fn decode_ber(reader: BERReader) -> ASN1Result<Self> {
        reader.read_sequence(|r| {
            let ca = r.read_default(false, |r| r.read_bool())?;
            let path_len_constraint = r.read_optional(|r| r.read_u64())?;
            Ok(BasicConstraints { ca, path_len_constraint })
        })
    }
}

#[deprecated(since="0.1.3", note="use `SubjectAltName` instead")]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct DnsAltNames<'a> {
    pub names: Vec<Cow<'a, str>>,
}

#[allow(deprecated)]
impl<'a> DerWrite for DnsAltNames<'a> {
    fn write(&self, writer: DERWriter) {
        writer.write_sequence(|writer| {
            for name in &self.names {
                writer.next().write_tagged_implicit(Tag::context(2), |w|
                    name.as_bytes().write(w)
                )
            }
        });
    }
}

#[allow(deprecated)]
impl<'a> BERDecodable for DnsAltNames<'a> {
    fn decode_ber(reader: BERReader) -> ASN1Result<Self> {
        reader.read_sequence(|seq_reader| {
            let mut names = Vec::<Cow<'a, str>>::new();

            loop {
                let res = seq_reader.read_optional(|r| {
                    r.read_tagged_implicit(Tag::context(2), |r| {
                        String::from_utf8(r.read_bytes()?).map_err(|_| ASN1Error::new(ASN1ErrorKind::Invalid))
                    })
                });
                match res {
                    Ok(Some(s)) => names.push(Cow::Owned(s)),
                    Ok(None) => break,
                    Err(e) => return Err(e),
                }
            }

            Ok(DnsAltNames { names })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubjectAltName<'a> {
    pub names: GeneralNames<'a>,
}

impl<'a> HasOid for SubjectAltName<'a> {
    fn oid() -> &'static ObjectIdentifier {
        &oid::subjectAltName
    }
}

impl<'a> DerWrite for SubjectAltName<'a> {
    fn write(&self, writer: DERWriter) {
        self.names.write(writer)
    }
}

impl<'a> BERDecodable for SubjectAltName<'a> {
    fn decode_ber(reader: BERReader) -> ASN1Result<Self> {
        Ok(SubjectAltName { names: GeneralNames::decode_ber(reader)? })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct IssuerAltName<'a> {
    pub names: GeneralNames<'a>,
}

impl<'a> HasOid for IssuerAltName<'a> {
    fn oid() -> &'static ObjectIdentifier {
        &oid::issuerAltName
    }
}

impl<'a> DerWrite for IssuerAltName<'a> {
    fn write(&self, writer: DERWriter) {
        self.names.write(writer)
    }
}

impl<'a> BERDecodable for IssuerAltName<'a> {
    fn decode_ber(reader: BERReader) -> ASN1Result<Self> {
        Ok(IssuerAltName { names: GeneralNames::decode_ber(reader)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::test_encode_decode;

    #[test]
    fn basic_constraints() {
        let test_values = vec![
            (BasicConstraints { ca: false, path_len_constraint: None    }, vec![0x30, 0x00]),
            (BasicConstraints { ca: true,  path_len_constraint: None    }, vec![0x30, 0x03, 0x01, 0x01, 0xFF]),
            (BasicConstraints { ca: true,  path_len_constraint: Some(4) }, vec![0x30, 0x06, 0x01, 0x01, 0xFF, 0x02, 0x01, 0x04]),
            // Not valid according to RFC 5280, but can still be decoded:
            (BasicConstraints { ca: false,  path_len_constraint: Some(4) }, vec![0x30, 0x03, 0x02, 0x01, 0x04]),
        ];

        for (basic_constraint, der) in test_values {
            test_encode_decode(&basic_constraint, &der);
        }
    }

    #[test]
    #[allow(deprecated)]
    fn dns_alt_names() {
        let names = DnsAltNames {
            names: vec![
                Cow::Borrowed("www.example.com"),
                Cow::Borrowed("example.com"),
            ]
        };

        let der = &[0x30, 0x1e, 0x82, 0x0f, 0x77, 0x77, 0x77, 0x2e, 0x65,
                    0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x63, 0x6f,
                    0x6d, 0x82, 0x0b, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c,
                    0x65, 0x2e, 0x63, 0x6f, 0x6d];

        test_encode_decode(&names, der);
    }

    #[test]
    fn subject_issuer_alt_name() {
        let subject_alt_name = SubjectAltName {
            names: GeneralNames(vec![
                GeneralName::IpAddress(vec![127,0,0,1]),
                GeneralName::RegisteredID(ObjectIdentifier::new(vec![1,2,3,4]))
            ]),
        };
        let issuer_alt_name = IssuerAltName {
            names: GeneralNames(vec![
                GeneralName::IpAddress(vec![127,0,0,1]),
                GeneralName::RegisteredID(ObjectIdentifier::new(vec![1,2,3,4]))
            ]),
        };

        let der = &[
            0x30, 0x0b, 0x87, 0x04, 0x7f, 0x00, 0x00, 0x01,
            0x88, 0x03, 0x2a, 0x03, 0x04];

        test_encode_decode(&subject_alt_name, der);
        test_encode_decode(&issuer_alt_name, der);
    }

    #[test]
    fn parse_v1_cert() {
        let der = include_bytes!("../tests/data/v1_cert.der");
        let cert = yasna::parse_der(der, |r| GenericCertificate::decode_ber(r)).unwrap();
        assert_eq!(cert.tbscert.version, TBS_CERTIFICATE_V1);
        assert_eq!(yasna::construct_der(|w| cert.write(w)), der);
    }

    #[test]
    fn parse_v3_cert() {
        let der = include_bytes!("../tests/data/v3_cert.der");
        let cert = yasna::parse_der(der, |r| GenericCertificate::decode_ber(r)).unwrap();
        assert_eq!(cert.tbscert.version, TBS_CERTIFICATE_V3);
        assert_eq!(yasna::construct_der(|w| cert.write(w)), der);
    }
}
