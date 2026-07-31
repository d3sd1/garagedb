//! Newtypes para identificadores. Nunca `String` a pelo en las APIs del dominio.

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.pad(&self.0) // respeta width/align en format!
            }
        }
    };
}

id_newtype!(ReplicaId);
id_newtype!(Sku);
id_newtype!(LocationId);
id_newtype!(MissionId);
id_newtype!(KitId);
id_newtype!(EventId);
