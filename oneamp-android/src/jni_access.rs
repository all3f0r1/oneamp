//! `AccessProvider` backed by a Kotlin `org.oneamp.AccessCallback`.
//! Called on decode workers, which attach to the JVM for the call.

use crate::wire::parse_access;
use jni::JavaVM;
use jni::objects::{GlobalRef, JObject, JString, JValue};
use oneamp_core::access::{Access, AccessProvider, RenewReason};
use oneamp_core::{ErrorCategory, PlayerError};
use std::sync::Arc;

pub struct JniAccess {
    pub vm: Arc<JavaVM>,
    pub callback: GlobalRef,
    pub media_id: String,
}

impl JniAccess {
    fn call(
        &self,
        method: &str,
        sig: &str,
        extra: Option<(&str, i64)>,
    ) -> Result<Access, PlayerError> {
        let fail = |what: String| PlayerError::new(ErrorCategory::AccessDenied, what);
        let mut env = self
            .vm
            .attach_current_thread()
            .map_err(|e| PlayerError::internal(format!("JVM attach: {e}")))?;
        let id = env
            .new_string(&self.media_id)
            .map_err(|e| PlayerError::internal(format!("JNI: {e}")))?;
        let result = match extra {
            None => env.call_method(&self.callback, method, sig, &[JValue::Object(&id)]),
            Some((reason, generation)) => {
                let reason = env
                    .new_string(reason)
                    .map_err(|e| PlayerError::internal(format!("JNI: {e}")))?;
                env.call_method(
                    &self.callback,
                    method,
                    sig,
                    &[
                        JValue::Object(&id),
                        JValue::Object(&reason),
                        JValue::Long(generation),
                    ],
                )
            }
        };
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err(fail(format!("AccessCallback.{method} threw")));
        }
        let obj: JObject = result
            .and_then(|v| v.l())
            .map_err(|e| PlayerError::internal(format!("AccessCallback.{method}: {e}")))?;
        if obj.is_null() {
            return Err(fail(format!("AccessCallback.{method} returned null")));
        }
        let s: String = env
            .get_string(&JString::from(obj))
            .map_err(|e| PlayerError::internal(format!("JNI: {e}")))?
            .into();
        parse_access(&s)
    }
}

impl AccessProvider for JniAccess {
    fn access(&self) -> Result<Access, PlayerError> {
        self.call("access", "(Ljava/lang/String;)Ljava/lang/String;", None)
    }

    fn renew(&self, rejected: &Access, reason: RenewReason) -> Result<Access, PlayerError> {
        let reason = match reason {
            RenewReason::Unauthorized => "unauthorized",
            RenewReason::Expired => "expired",
        };
        let mut a = self.call(
            "renew",
            "(Ljava/lang/String;Ljava/lang/String;J)Ljava/lang/String;",
            Some((reason, rejected.generation as i64)),
        )?;
        a.generation = rejected.generation;
        Ok(a)
    }
}
