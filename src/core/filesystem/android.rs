// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use jni::objects::{ JValue, JObject };
use jni::sys::jsize;
use std::collections::HashMap;
use super::*;
use crate::{ function_name, dbg_call };

pub fn get_jvm() -> jni::JavaVM {
    unsafe { jni::JavaVM::from_raw(ndk_context::android_context().vm().cast()) }.unwrap()
}

impl<'a> super::FileWrapper<'a> {
    pub fn open_android(jvm: &'a jni::JavaVM, url: &str, open_mode: &str) -> Result<Self> {
        let android_info = get_url_info(url)?;
        if let Some(size) = android_info.size {
            let handle = open_file(jvm, url, open_mode)?;
            return Ok(Self {
                file: Some(unsafe { std::os::fd::FromRawFd::from_raw_fd(handle.fd) }),
                size,
                url: url.to_owned(),
                android_handle: handle,
                _lifetime: Default::default()
            });
        }
        Err(FilesystemError::Unknown)
    }
}

pub struct AndroidFileHandle<'a> {
    _jvm: &'a jni::JavaVM,
    env: jni::AttachGuard<'a>,
    parcel: jni::objects::JObject<'a>,
    pub fd: i32,
}
impl<'a> Drop for AndroidFileHandle<'a> {
    fn drop(&mut self) {
        log::info!("Android close parcel");
        if let Err(e) = self.env.call_method(&self.parcel, "close", "()V", &[]) {
            log::warn!("Failed to close android file: {e:?}");
        }
    }
}

pub fn open_file<'a>(jvm: &'a jni::JavaVM, url: &str, open_mode: &str) -> Result<AndroidFileHandle<'a>> {
    dbg_call!(url open_mode);
    let mut env = jvm.attach_current_thread()?;
    let open_mode = env.new_string(open_mode)?;

    let uri = Uri::parse(&mut env, url)?;
    let resolver = ContentResolver::get(&mut env)?;
    let parcel = env.call_method(resolver, "openFileDescriptor", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;", &[
        JValue::Object(&uri),
        JValue::Object(&open_mode)
    ])?.l()?;

    let fd = env.call_method(&parcel, "getFd", "()I", &[])?.i()?;
    if fd <= 0 {
        log::error!("Failed to query android file descriptor: {fd}!");
        return Err(FilesystemError::InvalidFD(fd));
    }

    Ok(AndroidFileHandle {
        _jvm: jvm,
        env,
        fd,
        parcel
    })
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
    let mut ret = AndroidFileInfo::default();

    let vm = get_jvm();
    let mut env = vm.attach_current_thread()?;
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
    Ok(ret)
}

pub fn list_files(url: &str) -> Result<Vec<AndroidFileInfo>> {
    dbg_call!(url);
    if !url.starts_with("content://") || !is_dir_url(url) {
        return Err(FilesystemError::NotAFolder(url.into()));
    }
    let mut ret = Vec::new();

    let vm = get_jvm();
    let mut env = vm.attach_current_thread()?;
    let tree_uri = DocumentsContract::build_child_documents_uri_using_tree(&mut env, url)?;

    for x in ContentResolver::query(&mut env, &tree_uri, &["document_id", "_display_name", "mime_type", "_data", "_size"])? {
        let mut file = AndroidFileInfo::default();
        for (k, v) in x {
            match k.as_str() {
                "document_id"   => { file.url = Some(DocumentsContract::build_document_uri_using_tree(&mut env, &tree_uri, &v)?); }
                "_data"         => { file.path = Some(v); }
                "_display_name" => { file.filename = Some(v); }
                "mime_type"     => { file.is_dir = v == "vnd.android.document/directory"; }
                "_size"         => { file.size = Some(v.parse::<usize>().unwrap()); }
                _ => { panic!("Unhandled projection {k}"); }
            }
        }
        if file.filename.is_some() {
            if file.is_dir { file.size = None; }
            ret.push(file);
        }
    }

    Ok(ret)
}

pub fn create_file(tree_url: &str, filename: &str, mime_type: &str) -> Result<String> {
    dbg_call!(tree_url filename mime_type);
    if !tree_url.starts_with("content://") || !is_dir_url(tree_url) || filename.is_empty() {
        return Err(FilesystemError::InvalidPath(tree_url.into()));
    }

    let vm = get_jvm();
    let mut env = vm.attach_current_thread()?;
    let resolver = ContentResolver::get(&mut env)?;

    DocumentsContract::create_document(&mut env, &resolver, tree_url, filename, mime_type)
}

pub fn remove_file(url: &str) -> Result<bool> {
    dbg_call!(url);
    if !url.starts_with("content://") {
        return Err(FilesystemError::NotAFile(url.into()));
    }

    let vm = get_jvm();
    let mut env = vm.attach_current_thread()?;
    let resolver = ContentResolver::get(&mut env)?;

    DocumentsContract::delete_document(&mut env, &resolver, url)
}

pub fn is_dir_url(url: &str) -> bool {
    url.contains("/tree/") && !url.contains("/document/")
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Wrappers ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

struct Uri;
impl Uri {
    pub fn parse<'a>(env: &mut jni::AttachGuard<'a>, url: &str) -> Result<JObject<'a>> {
        let url = env.new_string(url)?;
        Ok(env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&url.into())])?.l()?)
    }
    pub fn to_string<'a>(env: &mut jni::AttachGuard<'a>, uri: &JObject<'a>) -> Result<String> {
        let uri_str = env.call_method(uri, "toString", "()Ljava/lang/String;", &[])?.l()?;
        Ok(unsafe { env.get_string_unchecked(&uri_str.into())?.into() })
    }
}

struct ContentResolver;
impl ContentResolver {
    pub fn get<'a>(env: &mut jni::AttachGuard<'a>) -> Result<JObject<'a>> {
        let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
        Ok(env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?)
    }
    pub fn query<'a>(env: &mut jni::AttachGuard<'a>, uri: &JObject<'a>, projections: &[&str]) -> Result<Vec<HashMap<String, String>>> {
        let resolver = Self::get(env)?;

        let mut projections_java = Vec::new();
        let projections_arr = env.new_object_array(projections.len() as jsize, "java/lang/String", JObject::null())?;
        for (i, arg) in projections.iter().enumerate() {
            projections_java.push(env.new_string(arg)?);
            env.set_object_array_element(&projections_arr, i as jsize, &projections_java[i])?;
        }

        let cursor = env.call_method(resolver, "query", "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;", &[
            JValue::Object(&uri),
            JValue::Object(&projections_arr),
            JValue::Object(&JObject::null()), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())
        ])?.l()?;

        let mut ret = Vec::new();

        if !cursor.as_raw().is_null() {
            while env.call_method(&cursor, "moveToNext", "()Z", &[])?.z()? {
                let mut map = HashMap::new();
                for (i, x) in projections.iter().enumerate() {
                    let column = env.call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&projections_java[i])])?.i()?;
                    if column > -1 {
                        match *x {
                            "_size" => {
                                let val = env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(column)])?.j()?;
                                map.insert(x.to_string(), format!("{}", val));
                            }
                            _ => {
                                let val = env.call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(column)])?.l()?;
                                if !val.as_raw().is_null() {
                                    map.insert(x.to_string(), unsafe { env.get_string_unchecked(&val.into())?.into() });
                                }
                            }
                        }
                    }
                }
                if !map.is_empty() {
                    ret.push(map);
                }
            }
            env.call_method(&cursor, "close", "()V", &[])?;
        } else {
            log::error!("query failed");
        }
        Ok(ret)
    }
}

struct DocumentsContract;
impl DocumentsContract {
    pub fn build_document_uri_using_tree<'a>(env: &mut jni::AttachGuard<'a>, tree_uri: &JObject<'a>, doc_id: &str) -> Result<String> {
        let doc_id = env.new_string(doc_id)?;
        let document_uri = env.call_static_method("android/provider/DocumentsContract", "buildDocumentUriUsingTree", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(tree_uri), JValue::Object(&doc_id)])?.l()?;

        Uri::to_string(env, &document_uri)
    }
    pub fn build_child_documents_uri_using_tree<'a>(env: &mut jni::AttachGuard<'a>, url: &str) -> Result<JObject<'a>> {
        let uri = Uri::parse(env, url)?;
        let doc_id = env.call_static_method("android/provider/DocumentsContract", "getTreeDocumentId", "(Landroid/net/Uri;)Ljava/lang/String;", &[JValue::Object(&uri)])?.l()?;
        let children_uri = env.call_static_method("android/provider/DocumentsContract", "buildChildDocumentsUriUsingTree", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&uri), JValue::Object(&doc_id)])?.l()?;

        Ok(children_uri)
    }
    pub fn delete_document<'a>(env: &mut jni::AttachGuard<'a>, resolver: &JObject<'a>, url: &str) -> Result<bool> {
        let uri = Uri::parse(env, url)?;
        Ok(env.call_static_method("android/provider/DocumentsContract", "deleteDocument", "(Landroid/content/ContentResolver;Landroid/net/Uri;)Z", &[
            JValue::Object(resolver),
            JValue::Object(&uri),
        ])?.z()?)
    }
    pub fn create_document<'a>(env: &mut jni::AttachGuard<'a>, resolver: &JObject<'a>, tree_url: &str, filename: &str, mime_type: &str) -> Result<String> {
        let filename = env.new_string(filename)?;
        let mime_type = env.new_string(mime_type)?;
        let tree_uri = DocumentsContract::build_child_documents_uri_using_tree(env, tree_url)?;

        let new_uri = env.call_static_method("android/provider/DocumentsContract", "createDocument", "(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;", &[
            JValue::Object(resolver),
            JValue::Object(&tree_uri),
            JValue::Object(&mime_type),
            JValue::Object(&filename),
        ])?.l()?;
        Uri::to_string(env, &new_uri)
    }
}