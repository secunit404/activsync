//! A FIT encoder that writes the same bytes as Garmin's Python FIT SDK
//! (garmin-fit-sdk 21.208), which the Python app uploads with: same header,
//! field order as given, local message reuse and CRC. Only the messages in
//! `assets/fit_profile.json` are known.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

const FIT_EPOCH_S: i64 = 631_065_600;
const HEADER_SIZE: usize = 14;
const MAX_FIELD_SIZE: usize = 255;
const CRC_TABLE: [u16; 16] = [
    0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800, 0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
];

#[derive(Deserialize)]
struct BaseType {
    size: usize,
    endian_flag: u8,
    float: bool,
    string: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Field {
    num: u8,
    base_type: u8,
    #[serde(rename = "type")]
    kind: String,
    scale: f64,
    offset: f64,
}

#[derive(Deserialize)]
struct Message {
    num: u16,
    fields: HashMap<String, Field>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    version: u16,
    base_types: HashMap<String, BaseType>,
    numeric_types: Vec<String>,
    messages: HashMap<String, Message>,
    types: HashMap<String, HashMap<String, String>>,
}

static PROFILE: LazyLock<Profile> =
    LazyLock::new(|| serde_json::from_str(include_str!("../assets/fit_profile.json")).expect("fit_profile.json parses"));

/// A field value as the SDK accepts it: a number, a profile enum by name, a
/// UTC instant in Unix seconds for `date_time` fields, text, or an array.
#[derive(Debug, Clone, PartialEq)]
pub enum FitValue {
    Int(i64),
    Float(f64),
    Name(&'static str),
    Time(i64),
    Text(String),
    List(Vec<FitValue>),
}

/// One message: its profile name and fields in the order they are written.
/// A field left out is simply absent, as the SDK drops `None`.
pub struct Mesg {
    pub name: &'static str,
    pub fields: Vec<(&'static str, FitValue)>,
}

pub fn mesg(name: &'static str, fields: Vec<(&'static str, Option<FitValue>)>) -> Mesg {
    Mesg { name, fields: fields.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect() }
}

#[derive(Clone, PartialEq)]
struct FieldDef {
    num: u8,
    size: usize,
    base_type: u8,
}

#[derive(Clone)]
struct Definition {
    global: u16,
    fields: Vec<FieldDef>,
}

impl Definition {
    /// The SDK compares definitions as sets of (num, size, base type), so a
    /// reordered message reuses the slot; kept for identical bytes.
    fn equals(&self, other: &Self) -> bool {
        self.global == other.global
            && self.fields.len() == other.fields.len()
            && self.fields.iter().all(|lhs| other.fields.iter().any(|rhs| lhs == rhs))
    }
}

fn py_round(value: f64) -> f64 {
    value.round_ties_even()
}

fn crc(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |mut crc, byte| {
        for nibble in [byte & 0x0F, byte >> 4] {
            let temp = CRC_TABLE[usize::from(crc & 0xF)];
            crc = ((crc >> 4) & 0x0FFF) ^ temp ^ CRC_TABLE[usize::from(nibble)];
        }
        crc
    })
}

enum Raw {
    Int(i64),
    Float(f64),
    Text(String),
}

pub struct Encoder {
    out: Vec<u8>,
    slots: [Option<Definition>; 16],
    next_local: usize,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self { out: vec![0; HEADER_SIZE], slots: Default::default(), next_local: 0 }
    }

    fn transform(value: &FitValue, field: &Field, name: &str) -> Result<Raw, String> {
        let profile = &*PROFILE;
        let fail = || format!("Could not convert \"{value:?}\" to \"{}\" for {name}", field.kind);
        if profile.numeric_types.contains(&field.kind) {
            let scaled = field.scale != 1.0 || field.offset != 0.0;
            let float_type = field.kind == "float32" || field.kind == "float64";
            return match value {
                FitValue::Int(v) if scaled && field.scale.fract() == 0.0 && field.offset.fract() == 0.0 => {
                    let raw = (*v + field.offset as i64) * field.scale as i64;
                    Ok(if float_type { Raw::Float(raw as f64) } else { Raw::Int(raw) })
                }
                FitValue::Int(v) if scaled => Self::scaled((*v as f64 + field.offset) * field.scale, float_type),
                FitValue::Float(v) if scaled => Self::scaled((v + field.offset) * field.scale, float_type),
                FitValue::Int(v) => Ok(Raw::Int(*v)),
                FitValue::Float(v) => Ok(Raw::Float(*v)),
                _ => Err(fail()),
            };
        }
        match field.kind.as_str() {
            "date_time" | "local_date_time" => match value {
                FitValue::Time(unix) => Ok(Raw::Int(unix - FIT_EPOCH_S)),
                FitValue::Int(v) => Ok(Raw::Int(*v)),
                FitValue::Float(v) => Ok(Raw::Float(*v)),
                _ => Err(fail()),
            },
            "string" => match value {
                FitValue::Text(text) => Ok(Raw::Text(text.clone())),
                _ => Err(fail()),
            },
            kind => match value {
                FitValue::Int(v) => Ok(Raw::Int(*v)),
                FitValue::Float(v) => Ok(Raw::Float(*v)),
                FitValue::Name(wanted) => profile
                    .types
                    .get(kind)
                    .and_then(|names| names.get(*wanted))
                    .and_then(|key| key.parse().ok())
                    .map(Raw::Int)
                    .ok_or_else(fail),
                _ => Err(fail()),
            },
        }
    }

    fn scaled(value: f64, float_type: bool) -> Result<Raw, String> {
        Ok(if float_type { Raw::Float(value) } else { Raw::Float(py_round(value)) })
    }

    fn write_raw(&mut self, raw: &Raw, base: &BaseType) {
        if base.string {
            if let Raw::Text(text) = raw {
                self.out.extend_from_slice(text.as_bytes());
            }
            self.out.push(0);
            return;
        }
        if base.float {
            let value = match raw {
                Raw::Int(v) => *v as f64,
                Raw::Float(v) => *v,
                Raw::Text(_) => 0.0,
            };
            if base.size == 4 {
                self.out.extend_from_slice(&(value as f32).to_le_bytes());
            } else {
                self.out.extend_from_slice(&value.to_le_bytes());
            }
            return;
        }
        // int(value) truncates toward zero, then the value is masked to the
        // field's width, exactly as the SDK packs it.
        let int = match raw {
            Raw::Int(v) => *v as i128,
            Raw::Float(v) => v.trunc() as i128,
            Raw::Text(_) => 0,
        };
        let bits = base.size * 8;
        let masked = (int & ((1i128 << bits) - 1)) as u128;
        self.out.extend_from_slice(&masked.to_le_bytes()[..base.size]);
    }

    pub fn write(&mut self, message: &Mesg) -> Result<(), String> {
        let profile = &*PROFILE;
        let definition = profile.messages.get(message.name).ok_or_else(|| format!("unknown FIT message {:?}", message.name))?;
        let mut fields = Vec::new();
        let mut defs = Vec::new();
        for (name, value) in &message.fields {
            let field =
                definition.fields.get(*name).ok_or_else(|| format!("FIT message {:?} has no field(s) [{name:?}]", message.name))?;
            let base = &profile.base_types[&field.base_type.to_string()];
            let values: Vec<&FitValue> = match value {
                FitValue::List(items) => items.iter().collect(),
                single => vec![single],
            };
            let size = if base.string {
                values.iter().map(|v| if let FitValue::Text(t) = v { t.len() + 1 } else { 1 }).sum()
            } else {
                base.size * values.len()
            };
            if size > MAX_FIELD_SIZE {
                return Err(format!("Some field sizes are greater than {MAX_FIELD_SIZE}"));
            }
            let raws = values.iter().map(|v| Self::transform(v, field, name)).collect::<Result<Vec<_>, _>>()?;
            defs.push(FieldDef { num: field.num, size, base_type: field.base_type });
            fields.push((raws, base));
        }
        if defs.is_empty() {
            return Err("No valid fields were found in the message".into());
        }
        let new = Definition { global: definition.num, fields: defs };

        let local = match self.slots.iter().position(|slot| slot.as_ref().is_some_and(|d| d.equals(&new))) {
            Some(index) => index,
            None => {
                let index = self.next_local & 0x0F;
                self.next_local += 1;
                index
            }
        };
        if !self.slots[local].as_ref().is_some_and(|d| d.equals(&new)) {
            self.out.extend_from_slice(&[0x40 | local as u8, 0, 0]);
            self.out.extend_from_slice(&new.global.to_le_bytes());
            self.out.push(new.fields.len() as u8);
            for def in &new.fields {
                let base = &profile.base_types[&def.base_type.to_string()];
                self.out.extend_from_slice(&[def.num, def.size as u8, def.base_type | base.endian_flag]);
            }
            self.slots[local] = Some(new);
        }
        self.out.push(local as u8);
        for (raws, base) in fields {
            for raw in &raws {
                self.write_raw(raw, base);
            }
        }
        Ok(())
    }

    pub fn close(mut self) -> Vec<u8> {
        let data_size = (self.out.len() - HEADER_SIZE) as u32;
        let mut header = [0u8; HEADER_SIZE];
        header[0] = HEADER_SIZE as u8;
        header[1] = 0x20;
        header[2..4].copy_from_slice(&PROFILE.version.to_le_bytes());
        header[4..8].copy_from_slice(&data_size.to_le_bytes());
        header[8..12].copy_from_slice(b".FIT");
        let header_crc = crc(&header[..12]);
        header[12..14].copy_from_slice(&header_crc.to_le_bytes());
        self.out[..HEADER_SIZE].copy_from_slice(&header);
        let file_crc = crc(&self.out);
        self.out.extend_from_slice(&file_crc.to_le_bytes());
        self.out
    }
}
