//! SPA params the backend reads (Props, EnumProfile/Profile, EnumRoute/Route) as plain data,
//! and the pods it writes.

use std::collections::BTreeMap;
use std::io::Cursor;

use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{CanonicalFixedSizedPod, ChoiceValue, Object, Pod, Property, Value, ValueArray};
use pipewire::spa::sys::*;
use pipewire::spa::utils::{Choice, ChoiceEnum};

pub const PROPS: u32 = SPA_PARAM_Props;
pub const ENUM_PROFILE: u32 = SPA_PARAM_EnumProfile;
pub const PROFILE: u32 = SPA_PARAM_Profile;
pub const ENUM_ROUTE: u32 = SPA_PARAM_EnumRoute;
pub const ROUTE: u32 = SPA_PARAM_Route;

/// Linear channel volumes, channel map and mute from a `Props` object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolumeProps {
    pub volumes: Vec<f32>,
    /// SPA channel positions.
    pub map: Vec<u32>,
    pub mute: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteParam {
    pub index: i32,
    pub direction: u32,
    /// The card profile device this route is active on (`Route` only).
    pub device: Option<i32>,
    pub name: String,
    pub description: String,
    pub priority: i32,
    pub available: u32,
    pub profiles: Vec<i32>,
    pub devices: Vec<i32>,
    pub props: Option<VolumeProps>,
    pub info: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileParam {
    pub index: i32,
    pub name: String,
    pub description: String,
    pub priority: i32,
    pub available: u32,
    /// How many sinks and sources the profile makes.
    pub sinks: u32,
    pub sources: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Props(VolumeProps),
    Profile(ProfileParam),
    Route(RouteParam),
}

/// Parses a param of type `id`; `None` for ones the backend doesn't use.
pub fn parse(id: u32, pod: &Pod) -> Option<Param> {
    let (_, value) = PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?;
    let Value::Object(obj) = value else { return None };
    match id {
        PROPS => Some(Param::Props(volume_props(&obj))),
        ENUM_PROFILE | PROFILE => Some(Param::Profile(profile(&obj))),
        ENUM_ROUTE | ROUTE => Some(Param::Route(route(&obj))),
        _ => None,
    }
}

fn field(obj: &Object, key: u32) -> Option<&Value> {
    obj.properties.iter().find(|p| p.key == key).map(|p| &p.value)
}

fn int(v: Option<&Value>) -> Option<i32> {
    match v? {
        Value::Int(i) => Some(*i),
        Value::Id(id) => Some(id.0 as i32),
        Value::Long(l) => Some(*l as i32),
        Value::Choice(ChoiceValue::Int(Choice(_, c))) => Some(choice_default(c)),
        Value::Choice(ChoiceValue::Id(Choice(_, c))) => Some(choice_default(c).0 as i32),
        _ => None,
    }
}

fn choice_default<T: CanonicalFixedSizedPod + Copy>(c: &ChoiceEnum<T>) -> T {
    match c {
        ChoiceEnum::None(v) => *v,
        ChoiceEnum::Range { default, .. } | ChoiceEnum::Step { default, .. } | ChoiceEnum::Enum { default, .. } | ChoiceEnum::Flags { default, .. } => *default,
    }
}

fn string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn ints(v: Option<&Value>) -> Vec<i32> {
    match v {
        Some(Value::ValueArray(ValueArray::Int(a))) => a.clone(),
        Some(Value::ValueArray(ValueArray::Id(a))) => a.iter().map(|i| i.0 as i32).collect(),
        _ => Vec::new(),
    }
}

/// `info` structs: `Struct { Int n, String k1, String v1, ... }`.
fn info(v: Option<&Value>) -> BTreeMap<String, String> {
    let Some(Value::Struct(items)) = v else { return BTreeMap::new() };
    let strings: Vec<&str> = items.iter().filter_map(|i| if let Value::String(s) = i { Some(s.as_str()) } else { None }).collect();
    strings.as_chunks::<2>().0.iter().map(|[k, v]| (k.to_string(), v.to_string())).collect()
}

pub fn volume_props(obj: &Object) -> VolumeProps {
    let volumes = match field(obj, SPA_PROP_channelVolumes) {
        Some(Value::ValueArray(ValueArray::Float(v))) => v.clone(),
        _ => Vec::new(),
    };
    let map = match field(obj, SPA_PROP_channelMap) {
        Some(Value::ValueArray(ValueArray::Id(v))) => v.iter().map(|i| i.0).collect(),
        Some(Value::ValueArray(ValueArray::Int(v))) => v.iter().map(|i| *i as u32).collect(),
        _ => Vec::new(),
    };
    let mute = match field(obj, SPA_PROP_mute) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    };
    VolumeProps { volumes, map, mute }
}

fn route(obj: &Object) -> RouteParam {
    RouteParam {
        index: int(field(obj, SPA_PARAM_ROUTE_index)).unwrap_or(-1),
        direction: int(field(obj, SPA_PARAM_ROUTE_direction)).unwrap_or(0) as u32,
        device: int(field(obj, SPA_PARAM_ROUTE_device)),
        name: string(field(obj, SPA_PARAM_ROUTE_name)),
        description: string(field(obj, SPA_PARAM_ROUTE_description)),
        priority: int(field(obj, SPA_PARAM_ROUTE_priority)).unwrap_or(0),
        available: int(field(obj, SPA_PARAM_ROUTE_available)).unwrap_or(0) as u32,
        profiles: ints(field(obj, SPA_PARAM_ROUTE_profiles)),
        devices: ints(field(obj, SPA_PARAM_ROUTE_devices)),
        props: match field(obj, SPA_PARAM_ROUTE_props) {
            Some(Value::Object(p)) => Some(volume_props(p)),
            _ => None,
        },
        info: info(field(obj, SPA_PARAM_ROUTE_info)),
    }
}

fn profile(obj: &Object) -> ProfileParam {
    let (mut sinks, mut sources) = (0, 0);
    // classes: Struct { Int n, Struct { String class, Int count, ... }, ... }
    if let Some(Value::Struct(items)) = field(obj, SPA_PARAM_PROFILE_classes) {
        for item in items {
            let Value::Struct(class) = item else { continue };
            let count = int(class.get(1)).unwrap_or(0).max(0) as u32;
            match class.first() {
                Some(Value::String(c)) if c == "Audio/Sink" => sinks += count,
                Some(Value::String(c)) if c == "Audio/Source" => sources += count,
                _ => {}
            }
        }
    }
    ProfileParam {
        index: int(field(obj, SPA_PARAM_PROFILE_index)).unwrap_or(-1),
        name: string(field(obj, SPA_PARAM_PROFILE_name)),
        description: string(field(obj, SPA_PARAM_PROFILE_description)),
        priority: int(field(obj, SPA_PARAM_PROFILE_priority)).unwrap_or(0),
        available: int(field(obj, SPA_PARAM_PROFILE_available)).unwrap_or(0) as u32,
        sinks,
        sources,
    }
}

// ---- building ------------------------------------------------------------------------------

pub fn serialize(value: &Value) -> Vec<u8> {
    PodSerializer::serialize(Cursor::new(Vec::new()), value).map(|(c, _)| c.into_inner()).unwrap_or_default()
}

/// A `Props` object with linear channel volumes and/or mute.
pub fn props_object(volumes: Option<&[f32]>, mute: Option<bool>) -> Object {
    let mut properties = Vec::new();
    if let Some(m) = mute {
        properties.push(Property::new(SPA_PROP_mute, Value::Bool(m)));
    }
    if let Some(v) = volumes {
        properties.push(Property::new(SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v.to_vec()))));
    }
    Object { type_: SPA_TYPE_OBJECT_Props, id: SPA_PARAM_Props, properties }
}

pub fn props_pod(volumes: Option<&[f32]>, mute: Option<bool>) -> Vec<u8> {
    serialize(&Value::Object(props_object(volumes, mute)))
}

/// A `Route` that WirePlumber saves (`save: true`), optionally carrying props.
pub fn route_pod(index: i32, device: i32, props: Option<Object>) -> Vec<u8> {
    let mut properties = vec![Property::new(SPA_PARAM_ROUTE_index, Value::Int(index)), Property::new(SPA_PARAM_ROUTE_device, Value::Int(device))];
    if let Some(p) = props {
        properties.push(Property::new(SPA_PARAM_ROUTE_props, Value::Object(p)));
    }
    properties.push(Property::new(SPA_PARAM_ROUTE_save, Value::Bool(true)));
    serialize(&Value::Object(Object { type_: SPA_TYPE_OBJECT_ParamRoute, id: SPA_PARAM_Route, properties }))
}

pub fn profile_pod(index: i32) -> Vec<u8> {
    serialize(&Value::Object(Object {
        type_: SPA_TYPE_OBJECT_ParamProfile,
        id: SPA_PARAM_Profile,
        properties: vec![Property::new(SPA_PARAM_PROFILE_index, Value::Int(index)), Property::new(SPA_PARAM_PROFILE_save, Value::Bool(true))],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipewire::spa::utils::Id;

    fn round_trip(id: u32, bytes: &[u8]) -> Param {
        parse(id, Pod::from_bytes(bytes).unwrap()).unwrap()
    }

    #[test]
    fn props_round_trip() {
        let bytes = props_pod(Some(&[0.5, 0.25]), Some(true));
        assert_eq!(round_trip(PROPS, &bytes), Param::Props(VolumeProps { volumes: vec![0.5, 0.25], map: vec![], mute: Some(true) }));
    }

    #[test]
    fn route_with_props_round_trips() {
        let bytes = route_pod(3, 1, Some(props_object(Some(&[1.0]), None)));
        let Param::Route(r) = round_trip(ROUTE, &bytes) else { panic!() };
        assert_eq!((r.index, r.device), (3, Some(1)));
        assert_eq!(r.props.unwrap().volumes, vec![1.0]);
    }

    #[test]
    fn enum_route_and_profile_parse() {
        let route = Value::Object(Object {
            type_: SPA_TYPE_OBJECT_ParamRoute,
            id: SPA_PARAM_EnumRoute,
            properties: vec![
                Property::new(SPA_PARAM_ROUTE_index, Value::Int(2)),
                Property::new(SPA_PARAM_ROUTE_direction, Value::Id(Id(SPA_DIRECTION_OUTPUT))),
                Property::new(SPA_PARAM_ROUTE_name, Value::String("analog-output-headphones".into())),
                Property::new(SPA_PARAM_ROUTE_description, Value::String("Headphones".into())),
                Property::new(SPA_PARAM_ROUTE_priority, Value::Int(9900)),
                Property::new(SPA_PARAM_ROUTE_available, Value::Id(Id(SPA_PARAM_AVAILABILITY_no))),
                Property::new(SPA_PARAM_ROUTE_info, Value::Struct(vec![Value::Int(1), Value::String("port.type".into()), Value::String("headphones".into())])),
                Property::new(SPA_PARAM_ROUTE_profiles, Value::ValueArray(ValueArray::Int(vec![1, 3]))),
                Property::new(SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(vec![0]))),
            ],
        });
        let Param::Route(r) = round_trip(ENUM_ROUTE, &serialize(&route)) else { panic!() };
        assert_eq!(r.name, "analog-output-headphones");
        assert_eq!(r.direction, SPA_DIRECTION_OUTPUT);
        assert_eq!(r.available, SPA_PARAM_AVAILABILITY_no);
        assert_eq!(r.profiles, vec![1, 3]);
        assert_eq!(r.devices, vec![0]);
        assert_eq!(r.device, None);
        assert_eq!(r.info.get("port.type").map(String::as_str), Some("headphones"));

        let class = |c: &str, n: i32| Value::Struct(vec![Value::String(c.into()), Value::Int(n), Value::String("card.profile.devices".into()), Value::ValueArray(ValueArray::Int(vec![0]))]);
        let profile = Value::Object(Object {
            type_: SPA_TYPE_OBJECT_ParamProfile,
            id: SPA_PARAM_EnumProfile,
            properties: vec![
                Property::new(SPA_PARAM_PROFILE_index, Value::Int(4)),
                Property::new(SPA_PARAM_PROFILE_name, Value::String("headset-head-unit-msbc".into())),
                Property::new(SPA_PARAM_PROFILE_description, Value::String("Headset Head Unit (HSP/HFP, codec mSBC)".into())),
                Property::new(SPA_PARAM_PROFILE_priority, Value::Int(30)),
                Property::new(SPA_PARAM_PROFILE_available, Value::Id(Id(SPA_PARAM_AVAILABILITY_yes))),
                Property::new(SPA_PARAM_PROFILE_classes, Value::Struct(vec![Value::Int(2), class("Audio/Sink", 1), class("Audio/Source", 1)])),
            ],
        });
        let Param::Profile(p) = round_trip(ENUM_PROFILE, &serialize(&profile)) else { panic!() };
        assert_eq!((p.index, p.sinks, p.sources, p.priority), (4, 1, 1, 30));
        assert_eq!(p.name, "headset-head-unit-msbc");
    }

    #[test]
    fn profile_pod_parses() {
        let Param::Profile(p) = round_trip(PROFILE, &profile_pod(7)) else { panic!() };
        assert_eq!(p.index, 7);
    }
}
