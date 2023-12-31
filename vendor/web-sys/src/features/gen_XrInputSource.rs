#![allow(unused_imports)]
#![allow(clippy::all)]
use super::*;
use wasm_bindgen::prelude::*;
#[cfg(web_sys_unstable_apis)]
#[wasm_bindgen]
extern "C" {
    # [wasm_bindgen (extends = :: js_sys :: Object , js_name = XRInputSource , typescript_type = "XRInputSource")]
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[doc = "The `XrInputSource` class."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrInputSource`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub type XrInputSource;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "XrHandedness")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = handedness)]
    #[doc = "Getter for the `handedness` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/handedness)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrHandedness`, `XrInputSource`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn handedness(this: &XrInputSource) -> XrHandedness;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "XrTargetRayMode")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = targetRayMode)]
    #[doc = "Getter for the `targetRayMode` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/targetRayMode)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrInputSource`, `XrTargetRayMode`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn target_ray_mode(this: &XrInputSource) -> XrTargetRayMode;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "XrSpace")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = targetRaySpace)]
    #[doc = "Getter for the `targetRaySpace` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/targetRaySpace)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrInputSource`, `XrSpace`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn target_ray_space(this: &XrInputSource) -> XrSpace;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "XrSpace")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = gripSpace)]
    #[doc = "Getter for the `gripSpace` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/gripSpace)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrInputSource`, `XrSpace`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn grip_space(this: &XrInputSource) -> Option<XrSpace>;
    #[cfg(web_sys_unstable_apis)]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = profiles)]
    #[doc = "Getter for the `profiles` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/profiles)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrInputSource`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn profiles(this: &XrInputSource) -> ::js_sys::Array;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "Gamepad")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = gamepad)]
    #[doc = "Getter for the `gamepad` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/gamepad)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `Gamepad`, `XrInputSource`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn gamepad(this: &XrInputSource) -> Option<Gamepad>;
    #[cfg(web_sys_unstable_apis)]
    #[cfg(feature = "XrHand")]
    # [wasm_bindgen (structural , method , getter , js_class = "XRInputSource" , js_name = hand)]
    #[doc = "Getter for the `hand` field of this object."]
    #[doc = ""]
    #[doc = "[MDN Documentation](https://developer.mozilla.org/en-US/docs/Web/API/XRInputSource/hand)"]
    #[doc = ""]
    #[doc = "*This API requires the following crate features to be activated: `XrHand`, `XrInputSource`*"]
    #[doc = ""]
    #[doc = "*This API is unstable and requires `--cfg=web_sys_unstable_apis` to be activated, as"]
    #[doc = "[described in the `wasm-bindgen` guide](https://rustwasm.github.io/docs/wasm-bindgen/web-sys/unstable-apis.html)*"]
    pub fn hand(this: &XrInputSource) -> Option<XrHand>;
}
