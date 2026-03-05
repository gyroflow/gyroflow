// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use jni::objects::{ JValue, JObject, JString };
use jni::{ jni_str, jni_sig };
use std::collections::HashMap;
use super::*;
use crate::{ function_name, dbg_call };

macro_rules! check_exception {
    ($env:expr, $typ:ty; $block:tt) => {{
        let res = (|| -> Result<$typ> {
            $block
        })();
        if res.is_err() {
            $env.exception_describe();
            $env.exception_clear();
        }
        res
    }}
}

pub fn get_jvm() -> jni::JavaVM {
    unsafe { jni::JavaVM::from_raw(ndk_context::android_context().vm().cast()) }
}

impl super::FileWrapper {
    pub fn open_android(url: &str, open_mode: &str) -> Result<Self> {
        let jvm = android::get_jvm();
        let android_info = get_url_info(url)?;
        if let Some(size) = android_info.size {
            let handle = open_file(&jvm, url, open_mode)?;
            return Ok(Self {
                file: Some(unsafe { std::os::fd::FromRawFd::from_raw_fd(handle.fd) }),
                size,
                url: url.to_owned(),
                android_handle: handle,
            });
        }
        Err(FilesystemError::Unknown)
    }
}

// 1. No local lifetimes attached to the struct
pub struct AndroidFileHandle {
    jvm: jni::JavaVM,                      // Store the JavaVM directly
    parcel: jni::refs::Global<JObject<'static>>, // Store a Global reference to survive the JNI frame
    pub fd: i32,
}

impl Drop for AndroidFileHandle {
    fn drop(&mut self) {
        log::info!("Closing android parcel");

        let obj = self.parcel.as_obj();

        // 2. Use attach_current_thread_for_scope inside Drop for best-effort cleanup
        let _ = self.jvm.attach_current_thread_for_scope(|env| {
            // Using standard JNI void signature "()V"
            if let Err(e) = env.call_method(obj, jni_str!("close"), jni_sig!("()V"), &[]) {
                log::warn!("Failed to close android parcel: {e:?}");

                // Manually clear exceptions inside the Drop block to keep JVM state clean
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            Ok::<(), jni::errors::Error>(())
        });
    }
}

pub fn open_file(jvm: &jni::JavaVM, url: &str, open_mode: &str) -> Result<AndroidFileHandle> {
    dbg_call!(url open_mode);

    // 3. Callback-based `attach_current_thread` as required in 0.22
    Ok(jvm.attach_current_thread(|mut env| {
        let open_mode_jstr = env.new_string(open_mode)?;
        let uri = Uri::parse(&mut env, url)?;
        let resolver = ContentResolver::get(&mut env)?;

        let parcel = env.call_method(
            resolver,
            jni_str!("openFileDescriptor"),
            jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;"),
            &[
                JValue::Object(&uri),
                JValue::Object(&open_mode_jstr)
            ]
        )?.l()?;

        // Standard JNI int signature "()I"
        let fd = env.call_method(&parcel, jni_str!("getFd"), jni_sig!("()I"), &[])?.i()?;

        if fd <= 0 {
            log::error!("Failed to query android file descriptor: {fd}!");
            return Err(FilesystemError::InvalidFD(fd));
        }

        // 4. Upgrade the local reference to a Global reference
        let global_parcel = env.new_global_ref(parcel)?;

        Ok::<AndroidFileHandle, FilesystemError>(AndroidFileHandle {
            jvm: jvm.clone(), // Cheap clone of the internal JVM pointer
            fd,
            parcel: global_parcel,
        })
    })?)
}

#[derive(Debug, Default, Clone)]
pub struct AndroidFileInfo {
    pub filename: Option<String>,
    pub size: Option<usize>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub is_dir: bool
}

pub fn get_url_info(url: &str) -> Result<AndroidFileInfo> {
    dbg_call!(url);
    if !url.starts_with("content://") {
        return Err(FilesystemError::InvalidPath(url.to_owned()));
    }
    let vm = get_jvm();
    Ok(vm.attach_current_thread(|mut env| {
        check_exception!(env, AndroidFileInfo; {
            let mut ret = AndroidFileInfo::default();

            let uri = Uri::parse(&mut env, url)?;

            for x in ContentResolver::query(&mut env, &uri, &["_display_name", "_size", "_data", "mime_type"])? {
                for (k, v) in x {
                    match k.as_str() {
                        "_data"         => { ret.path = Some(v); }
                        "_display_name" => { ret.filename = Some(v); ret.url = Some(url.to_string()); }
                        "mime_type"     => { ret.is_dir = v == "vnd.android.document/directory"; }
                        "_size"         => { ret.size = Some(v.parse::<usize>().unwrap()); }
                        _ => { panic!("Unhandled projection {k}"); }
                    }
                }
            }
            if ret.is_dir { ret.size = None; }
            log::debug!("get_url_info: {ret:?}");
            Ok(ret)
        })
    })?)
}

pub fn list_files(url: &str) -> Result<Vec<AndroidFileInfo>> {
    dbg_call!(url);
    if !url.starts_with("content://") || !is_dir_url(url) {
        return Err(FilesystemError::NotAFolder(url.into()));
    }
    let vm = get_jvm();
    Ok(vm.attach_current_thread(|mut env| {
        check_exception!(env, Vec<AndroidFileInfo>; {
            let mut ret = Vec::new();

            let tree_uri = if url.contains("/children") {
                Uri::parse(&mut env, &url)?
            } else {
                DocumentsContract::build_child_documents_uri_using_tree(&mut env, url)?
            };

            for x in ContentResolver::query(&mut env, &tree_uri, &["mime_type", "document_id", "_display_name", "_data", "_size"])? {
                let mut file = AndroidFileInfo::default();
                let mut document_id = None;
                for (k, v) in x {
                    match k.as_str() {
                        "document_id"   => { document_id = Some(v); }
                        "_data"         => { file.path = Some(v); }
                        "_display_name" => { file.filename = Some(v); }
                        "mime_type"     => { file.is_dir = v == "vnd.android.document/directory"; }
                        "_size"         => { file.size = Some(v.parse::<usize>().unwrap()); }
                        _ => { panic!("Unhandled projection {k}"); }
                    }
                }
                if let Some(document_id) = document_id {
                    file.url = Some(if file.is_dir {
                        DocumentsContract::build_children_uri_using_tree(&mut env, &tree_uri, &document_id)?
                    } else {
                        DocumentsContract::build_document_uri_using_tree(&mut env, &tree_uri, &document_id)?
                    });
                }
                if file.filename.is_some() {
                    if file.is_dir { file.size = None; }
                    ret.push(file);
                }
            }

            Ok(ret)
        })
    })?)
}

pub fn create_file(tree_url: &str, filename: &str, mime_type: &str) -> Result<String> {
    dbg_call!(tree_url filename mime_type);
    if !tree_url.starts_with("content://") || !is_dir_url(tree_url) || filename.is_empty() {
        return Err(FilesystemError::InvalidPath(tree_url.into()));
    }
    let vm = get_jvm();
    Ok(vm.attach_current_thread(|mut env| {
        check_exception!(env, String; {
            let resolver = ContentResolver::get(&mut env)?;

            DocumentsContract::create_document(&mut env, &resolver, tree_url, filename, mime_type)
        })
    })?)
}

pub fn remove_file(url: &str) -> Result<bool> {
    dbg_call!(url);
    if !url.starts_with("content://") {
        return Err(FilesystemError::NotAFile(url.into()));
    }
    let vm = get_jvm();
    Ok(vm.attach_current_thread(|mut env| {
        check_exception!(env, bool; {
            let resolver = ContentResolver::get(&mut env)?;

            DocumentsContract::delete_document(&mut env, &resolver, url)
        })
    })?)
}

pub fn is_dir_url(url: &str) -> bool {
    fn inner(url: &str) -> bool {
        if !url.contains("/tree/")   { return false; }
        if url.ends_with("/")        { return true; }
        if url.contains("/children") { return true; }

        match get_url_info(url) {
            Ok(x) => x.is_dir,
            Err(_e) => {
                match get_url_info(&format!("{url}/")) {
                    Ok(x) => x.is_dir,
                    Err(e) => {
                        log::error!("Failed to get url info for {url}: {e:?}");
                        // Check if the file has extension - not ideal but should work for most cases
                        // FIXME: write this properly
                        url.split('/').last().map(|x| !x.contains('.')).unwrap_or(false)
                    }
                }
            }
        }
    }

    let ret = inner(url);
    dbg_call!(url -> ret);
    ret
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Wrappers ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

struct Uri;
impl Uri {
    pub fn parse<'a>(env: &mut jni::Env<'a>, url: &str) -> Result<JObject<'a>> {
        let url = if url.contains(' ') {
            url.replace(' ', "%20")
        } else {
            url.to_owned()
        };

        let url = env.new_string(&url)?;
        Ok(env.call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&url.into())])?.l()?)
    }
    pub fn to_string<'a>(env: &mut jni::Env<'a>, uri: &JObject<'a>) -> Result<String> {
        let uri_str = env.call_method(uri, jni_str!("toString"), jni_sig!(() -> JString), &[])?.l()?;
        Ok(env.as_cast::<JString>(&uri_str)?.to_string())
    }
    pub fn get_authority<'a>(env: &mut jni::Env<'a>, uri: &JObject<'a>) -> Result<JObject<'a>> {
        Ok(env.call_method(uri, jni_str!("getAuthority"), jni_sig!(() -> JString), &[])?.l()?)
    }
}

struct ContentResolver;
impl ContentResolver {
    pub fn get<'a>(env: &mut jni::Env<'a>) -> Result<JObject<'a>> {
        let context = unsafe { JObject::from_raw(env, ndk_context::android_context().context().cast()) };
        Ok(env.call_method(context, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[])?.l()?)
    }
    pub fn query<'a>(env: &mut jni::Env<'a>, uri: &JObject<'a>, projections: &[&str]) -> Result<Vec<HashMap<String, String>>> {
        let resolver = Self::get(env)?;

        let mut projections_java = Vec::new();
        let projections_arr = jni::objects::JObjectArray::<JString>::new(env, projections.len(), JString::null())?;
        for (i, arg) in projections.iter().enumerate() {
            projections_java.push(JString::from_str(env, arg)?);
            projections_arr.set_element(env, i as _, &projections_java[i])?;
        }

        let cursor = env.call_method(resolver, jni_str!("query"), jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"), &[
            JValue::Object(&uri),
            JValue::Object(&projections_arr),
            JValue::Object(&JObject::null()), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())
        ])?.l()?;

        let mut ret = Vec::new();

        if !cursor.as_raw().is_null() {
            while env.call_method(&cursor, jni_str!("moveToNext"), jni_sig!("()Z"), &[])?.z()? {
                let mut map = HashMap::new();
                for (i, x) in projections.iter().enumerate() {
                    let column = env.call_method(&cursor, jni_str!("getColumnIndex"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&projections_java[i])])?.i()?;
                    if column > -1 {
                        match *x {
                            "_size" => {
                                let val = env.call_method(&cursor, jni_str!("getLong"), jni_sig!("(I)J"), &[JValue::Int(column)])?.j()?;
                                map.insert(x.to_string(), format!("{}", val));
                            }
                            _ => {
                                let val = env.call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::Int(column)])?.l()?;
                                if !val.as_raw().is_null() {
                                    map.insert(x.to_string(), env.as_cast::<JString>(&val)?.to_string());
                                }
                            }
                        }
                    }
                }
                if !map.is_empty() {
                    ret.push(map);
                }
            }
            env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[])?;
        } else {
            log::error!("query failed");
        }
        Ok(ret)
    }
}

struct DocumentsContract;
impl DocumentsContract {
    pub fn build_document_uri_using_tree<'a>(env: &mut jni::Env<'a>, tree_uri: &JObject<'a>, doc_id: &str) -> Result<String> {
        let doc_id = env.new_string(doc_id)?;
        let document_uri = env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("buildDocumentUriUsingTree"), jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(tree_uri), JValue::Object(&doc_id)])?.l()?;

        Uri::to_string(env, &document_uri)
    }
    pub fn build_children_uri_using_tree<'a>(env: &mut jni::Env<'a>, tree_uri: &JObject<'a>, doc_id: &str) -> Result<String> {
        let authority = Uri::get_authority(env, tree_uri)?;
        let doc_id = env.new_string(doc_id)?;
        let document_uri = env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("buildChildDocumentsUri"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&authority), JValue::Object(&doc_id)])?.l()?;

        Uri::to_string(env, &document_uri)
    }
    pub fn build_child_documents_uri_using_tree<'a>(env: &mut jni::Env<'a>, url: &str) -> Result<JObject<'a>> {
        let uri = Uri::parse(env, url)?;
        let doc_id = env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("getTreeDocumentId"), jni_sig!("(Landroid/net/Uri;)Ljava/lang/String;"), &[JValue::Object(&uri)])?.l()?;
        let children_uri = env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("buildChildDocumentsUriUsingTree"), jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&uri), JValue::Object(&doc_id)])?.l()?;

        Ok(children_uri)
    }
    /*pub fn get_tree_document_id<'a>(env: &mut jni::Env<'a>, url: &str) -> Result<String> {
        let uri = Uri::parse(env, url)?;
        let doc_id = env.call_static_method("android/provider/DocumentsContract", "getTreeDocumentId", "(Landroid/net/Uri;)Ljava/lang/String;", &[JValue::Object(&uri)])?.l()?;
        Ok(unsafe { env.get_string_unchecked(&doc_id.into())?.into() })
    }
    pub fn is_tree_uri<'a>(env: &mut jni::Env<'a>, url: &str) -> Result<bool> {
        let uri = Uri::parse(env, url)?;
        Ok(env.call_static_method("android/provider/DocumentsContract", "isTreeUri", "(Landroid/net/Uri;)Z", &[JValue::Object(&uri)])?.z()?)
    }
    pub fn is_document_uri<'a>(env: &mut jni::Env<'a>, url: &str) -> Result<bool> {
        let uri = Uri::parse(env, url)?;
        Ok(env.call_static_method("android/provider/DocumentsContract", "isDocumentUri", "(Landroid/net/Uri;)Z", &[JValue::Object(&uri)])?.z()?)
    }*/
    pub fn delete_document<'a>(env: &mut jni::Env<'a>, resolver: &JObject<'a>, url: &str) -> Result<bool> {
        let uri = Uri::parse(env, url)?;
        Ok(env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("deleteDocument"), jni_sig!("(Landroid/content/ContentResolver;Landroid/net/Uri;)Z"), &[
            JValue::Object(resolver),
            JValue::Object(&uri),
        ])?.z()?)
    }
    pub fn create_document<'a>(env: &mut jni::Env<'a>, resolver: &JObject<'a>, tree_url: &str, filename: &str, mime_type: &str) -> Result<String> {
        let filename = env.new_string(filename)?;
        let mime_type = env.new_string(mime_type)?;
        let tree_uri = DocumentsContract::build_child_documents_uri_using_tree(env, tree_url)?;

        let new_uri = env.call_static_method(jni_str!("android/provider/DocumentsContract"), jni_str!("createDocument"), jni_sig!("(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;"), &[
            JValue::Object(resolver),
            JValue::Object(&tree_uri),
            JValue::Object(&mime_type),
            JValue::Object(&filename),
        ])?.l()?;
        Uri::to_string(env, &new_uri)
    }
}
