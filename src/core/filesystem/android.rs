// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use jni::objects::{ JValue, JObject };
use jni::sys::jsize;
use super::*;
use crate::{ function_name, dbg_call };

pub fn get_jvm() -> jni::JavaVM {
    unsafe { jni::JavaVM::from_raw(ndk_context::android_context().vm().cast()) }.unwrap()
}

impl<'a> super::FileWrapper<'a> {
    pub fn open_android(jvm: &'a jni::JavaVM, url: &str, open_mode: &str) -> std::result::Result<Self, FilesystemError> {
        let android_info = get_url_info(jvm, url)?;
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

pub fn open_file<'a>(jvm: &'a jni::JavaVM, url: &str, open_mode: &str) -> std::result::Result<AndroidFileHandle<'a>, FilesystemError> {
    dbg_call!(url open_mode);
    let mut env = jvm.attach_current_thread()?;
    let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
    let url = env.new_string(url)?;
    let open_mode = env.new_string(open_mode)?;

    let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&url.into())])?.l()?;
    let resolver = env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let parcel = env.call_method(resolver, "openFileDescriptor", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;", &[
        JValue::Object(&uri),
        JValue::Object(&open_mode.into())
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

pub fn get_url_info(vm: &jni::JavaVM, url: &str) -> std::result::Result<AndroidFileInfo, FilesystemError> {
    dbg_call!(url);
    let mut ret = AndroidFileInfo::default();
    if !url.starts_with("content://") {
        return Err(FilesystemError::InvalidPath(url.to_owned()));
    }
    let projections = ["_display_name", "_size", "_data", "mime_type"];
    let mut projections_java = Vec::new();

    let mut env = vm.attach_current_thread()?;

    let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
    let url_copy = url.to_owned();
    let url = env.new_string(url)?;
    let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&url.into())])?.l()?;

    let projections_arr = env.new_object_array(projections.len() as jsize, "java/lang/String", JObject::null())?;
    for (i, arg) in projections.iter().enumerate() {
        projections_java.push(env.new_string(arg)?);
        env.set_object_array_element(&projections_arr, i as jsize, &projections_java[i])?;
    }

    let resolver = env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let cursor = env.call_method(resolver, "query", "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;", &[
        JValue::Object(&uri),
        JValue::Object(&projections_arr),
        JValue::Object(&JObject::null()), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())
    ])?.l()?;

    if !cursor.as_raw().is_null() {
        if env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
            for (i, x) in projections.iter().enumerate() {
                let column = env.call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&projections_java[i])])?.i()?;
                if column > -1 {
                    match *x {
                        "_display_name" | "_data" | "mime_type" => {
                            let val = env.call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(column)])?.l()?;
                            if !val.as_raw().is_null() {
                                let val: String = unsafe { env.get_string_unchecked(&val.into())?.into() };
                                match *x {
                                    "_display_name" => { ret.filename = Some(val); ret.url = Some(url_copy.clone()); }
                                    "_data" => { ret.path = Some(val); }
                                    "mime_type" => { ret.is_dir = val == "vnd.android.document/directory"; }
                                    _ => { }
                                }
                            }
                        },
                        "_size" => {
                            let val = env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(column)])?.j()?;
                            ret.size = Some(val as usize);
                        }
                        _ => { panic!("Unhandled projection: {x} "); }
                    }
                }
            }
        }
        env.call_method(&cursor, "close", "()V", &[])?;
    } else {
        log::error!("query failed");
    }

    Ok(ret)
}

// fn java_string_array(arr: &[&str]) -> std::result::Result<Vec<AndroidFileInfo>, jni::errors::Error>

pub fn list_files(vm: &jni::JavaVM, url: &str) -> std::result::Result<Vec<AndroidFileInfo>, FilesystemError> {
    dbg_call!(url);
    let mut ret = Vec::new();
    if !url.starts_with("content://") || !is_dir_url(url) {
        return Err(FilesystemError::NotAFolder(url.into()));
    }
    let projections = ["document_id", "_display_name", "mime_type", "_data", "_size"];
    let mut projections_java = Vec::new();

    let mut env = vm.attach_current_thread()?;

    let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
    let url = env.new_string(url)?;
    let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&url.into())])?.l()?;
    let doc_id = env.call_static_method("android/provider/DocumentsContract", "getTreeDocumentId", "(Landroid/net/Uri;)Ljava/lang/String;", &[JValue::Object(&uri)])?.l()?;
    let children_uri = env.call_static_method("android/provider/DocumentsContract", "buildChildDocumentsUriUsingTree", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&uri), JValue::Object(&doc_id)])?.l()?;

    let projections_arr = env.new_object_array(projections.len() as jsize, "java/lang/String", JObject::null())?;
    for (i, arg) in projections.iter().enumerate() {
        projections_java.push(env.new_string(arg)?);
        env.set_object_array_element(&projections_arr, i as jsize, &projections_java[i])?;
    }

    let resolver = env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let cursor = env.call_method(resolver, "query", "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;", &[
        JValue::Object(&children_uri),
        JValue::Object(&projections_arr),
        JValue::Object(&JObject::null()), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())
    ])?.l()?;

    if !cursor.as_raw().is_null() {
        while env.call_method(&cursor, "moveToNext", "()Z", &[])?.z()? {
            let mut file = AndroidFileInfo::default();
            for (i, x) in projections.iter().enumerate() {
                let column = env.call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&projections_java[i])])?.i()?;
                if column > -1 {
                    match *x {
                        "document_id" | "_display_name" | "mime_type" | "_data" => {
                            let val = env.call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(column)])?.l()?;
                            if !val.as_raw().is_null() {
                                match *x {
                                    "document_id" => {
                                        let document_uri = env.call_static_method("android/provider/DocumentsContract", "buildDocumentUriUsingTree", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&children_uri), JValue::Object(&val)])?.l()?;
                                        let document_uri = env.call_method(document_uri, "toString", "()Ljava/lang/String;", &[])?.l()?;
                                        file.url = Some(unsafe { env.get_string_unchecked(&document_uri.into())?.into() });
                                    }
                                    "_data"         => { file.path = Some(unsafe { env.get_string_unchecked(&val.into())?.into() }); }
                                    "_display_name" => { file.filename = Some(unsafe { env.get_string_unchecked(&val.into())?.into() }); }
                                    "mime_type"     => { file.is_dir = unsafe { env.get_string_unchecked(&val.into())?.to_string_lossy() } == "vnd.android.document/directory"; }
                                    _ => { }
                                }
                            }
                        },
                        "_size" => {
                            let val = env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(column)])?.j()?;
                            if !file.is_dir {
                                file.size = Some(val as usize);
                            }
                        }
                        _ => { panic!("Unhandled projection: {x} "); }
                    }
                }
            }
            if file.filename.is_some() {
                ret.push(file);
            }
        }
        env.call_method(&cursor, "close", "()V", &[])?;
    } else {
        log::error!("query failed");
    }

    Ok(ret)
}

pub fn create_file(vm: &jni::JavaVM, tree_url: &str, filename: &str, mime_type: &str) -> std::result::Result<String, FilesystemError> {
    dbg_call!(tree_url filename mime_type);
    let mut ret = String::new();
    if !tree_url.starts_with("content://") || !is_dir_url(tree_url) || filename.is_empty() {
        return Err(FilesystemError::InvalidPath(tree_url.into()));
    }

    let mut env = vm.attach_current_thread()?;

    let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
    let resolver = env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let tree_url = env.new_string(tree_url)?;
    let filename = env.new_string(filename)?;
    let mime_type = env.new_string(mime_type)?;
    let tree_uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&tree_url.into())])?.l()?;
    let doc_id = env.call_static_method("android/provider/DocumentsContract", "getTreeDocumentId", "(Landroid/net/Uri;)Ljava/lang/String;", &[JValue::Object(&tree_uri)])?.l()?;
    let tree_uri = env.call_static_method("android/provider/DocumentsContract", "buildChildDocumentsUriUsingTree", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&tree_uri), JValue::Object(&doc_id)])?.l()?;

    let new_uri = env.call_static_method("android/provider/DocumentsContract", "createDocument", "(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;", &[
        JValue::Object(&resolver),
        JValue::Object(&tree_uri),
        JValue::Object(&mime_type),
        JValue::Object(&filename),
    ])?.l()?;
    let new_uri = env.call_method(new_uri, "toString", "()Ljava/lang/String;", &[])?.l()?;
    Ok(unsafe { env.get_string_unchecked(&new_uri.into())?.into() })
}

pub fn remove_file(vm: &jni::JavaVM, url: &str) -> std::result::Result<bool, FilesystemError> {
    dbg_call!(url);
    if !url.starts_with("content://") {
        return Err(FilesystemError::NotAFile(url.into()));
    }
    let mut env = vm.attach_current_thread()?;

    let context = unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) };
    let resolver = env.call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let url = env.new_string(url)?;
    let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&url.into())])?.l()?;
    Ok(env.call_static_method("android/provider/DocumentsContract", "deleteDocument", "(Landroid/content/ContentResolver;Landroid/net/Uri;)Z", &[
        JValue::Object(&resolver),
        JValue::Object(&uri),
    ])?.z()?)
}

pub fn is_dir_url(url: &str) -> bool {
    url.contains("/tree/") && !url.contains("/document/")
}
