use serde::{de::DeserializeOwned, Serialize};

pub fn assert_toml_roundtrip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let serialized = toml::to_string(value).expect("Failed to serialize to TOML");
    let deserialized: T = toml::from_str(&serialized).expect("Failed to deserialize from TOML");
    assert_eq!(
        value, &deserialized,
        "Round-trip serialization failed.\nOriginal: {:?}\nDeserialized: {:?}\nSerialized TOML:\n{}",
        value, deserialized, serialized
    );
}
