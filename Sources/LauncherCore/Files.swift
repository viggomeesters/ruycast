import Foundation
import Darwin
import LauncherModels

public enum Files {
    public static func encode<T:Encodable>(_ value:T) throws -> Data {let e=JSONEncoder();e.outputFormatting=[.prettyPrinted,.sortedKeys,.withoutEscapingSlashes];e.dateEncodingStrategy = .iso8601;return try e.encode(value)}
    public static func decode<T:Decodable>(_ type:T.Type,data:Data) throws -> T {let d=JSONDecoder();d.dateDecodingStrategy = .iso8601;return try d.decode(type,from:data)}
    public static func config(_ path:URL) throws -> Configuration {try config(data:Data(contentsOf:path))}
    public static func config(data:Data) throws -> Configuration {let raw=try JSONSerialization.jsonObject(with:data);return try RustCore.decode(Configuration.self,op:"validate",payload:raw)}
    /// The selected local profile is private; preserve supported path aliases.
    public static func securePrivateDirectory(_ url:URL) throws {
        try FileManager.default.createDirectory(at:url,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        func failure(_ code:Int32=errno)->NSError {NSError(domain:NSPOSIXErrorDomain,code:Int(code),userInfo:[NSFilePathErrorKey:url.path])}
        let fd=open(url.path,O_RDONLY|O_DIRECTORY|O_CLOEXEC)
        guard fd>=0 else {throw failure()};defer {close(fd)}
        var info=stat()
        guard fstat(fd,&info)==0 else {throw failure()}
        guard (info.st_mode & S_IFMT)==S_IFDIR,info.st_uid==getuid() else {throw failure(EACCES)}
        guard fchmod(fd,0o700)==0 else {throw failure()}
        guard let acl=acl_init(0) else {throw failure()};defer {acl_free(UnsafeMutableRawPointer(acl))}
        guard acl_set_fd_np(fd,acl,ACL_TYPE_EXTENDED)==0 else {throw failure()}
        guard fstat(fd,&info)==0,(info.st_mode & S_IFMT)==S_IFDIR,info.st_uid==getuid(),info.st_mode & 0o777==0o700 else {throw failure()}
        if let current=acl_get_fd_np(fd,ACL_TYPE_EXTENDED) {
            defer {acl_free(UnsafeMutableRawPointer(current))};var entry:acl_entry_t?
            guard acl_valid(current)==0,acl_get_entry(current,Int32(ACL_FIRST_ENTRY.rawValue),&entry) != 0 else {throw failure()}
        } else if errno != ENOENT {throw failure()}
    }
    public static func save<T:Encodable>(_ value:T,to url:URL) throws {try savePrivate(encode(value),to:url)}
    /// Private from creation, then atomically replace the destination. Parent
    /// permissions stay unchanged for deliberately shared/synced config folders.
    public static func savePrivate(_ bytes:Data,to url:URL) throws {try writePrivate(bytes,to:url,replace:true,synchronize:false)}
    public static func createPrivate(_ bytes:Data,to url:URL) throws {try writePrivate(bytes,to:url,replace:false,synchronize:true)}
    private static func writePrivate(_ bytes:Data,to url:URL,replace:Bool,synchronize:Bool) throws {
        let parent=url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at:parent,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        func failure(_ code:Int32=errno)->NSError {NSError(domain:NSPOSIXErrorDomain,code:Int(code),userInfo:[NSFilePathErrorKey:url.path])}
        let directory=open(parent.path,O_RDONLY|O_DIRECTORY|O_CLOEXEC)
        guard directory>=0 else {throw failure()};defer {close(directory)}
        // A file opened before its inherited ACL is cleared would retain read
        // access. Secure an empty staging directory first, then create payload.
        let temporary=".ruycast-"+UUID().uuidString+".tmp"
        guard mkdirat(directory,temporary,0o700)==0 else {throw failure()}
        var created=stat()
        guard fstatat(directory,temporary,&created,AT_SYMLINK_NOFOLLOW)==0,
              (created.st_mode & S_IFMT)==S_IFDIR,created.st_uid==getuid() else {throw failure()}
        var staging:Int32 = -1
        defer {
            var current=stat()
            if fstatat(directory,temporary,&current,AT_SYMLINK_NOFOLLOW)==0,
               (current.st_mode & S_IFMT)==S_IFDIR,current.st_dev==created.st_dev,current.st_ino==created.st_ino {
                unlinkat(directory,temporary,AT_REMOVEDIR)
            }
            if staging>=0 {close(staging)}
        }
        staging=openat(directory,temporary,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)
        guard staging>=0 else {throw failure()}
        var stageInfo=stat()
        guard fstat(staging,&stageInfo)==0,stageInfo.st_uid==getuid(),
              stageInfo.st_dev==created.st_dev,stageInfo.st_ino==created.st_ino else {throw failure()}
        guard fchmod(staging,0o700)==0 else {throw failure()}
        guard let acl=acl_init(0) else {throw failure()};defer {acl_free(UnsafeMutableRawPointer(acl))}
        guard acl_set_fd_np(staging,acl,ACL_TYPE_EXTENDED)==0 else {throw failure()}
        func verifyPrivate(_ fd:Int32,mode:mode_t) throws {
            var info=stat()
            guard fstat(fd,&info)==0,info.st_uid==getuid(),info.st_mode & 0o777==mode else {throw failure(EACCES)}
            if let current=acl_get_fd_np(fd,ACL_TYPE_EXTENDED) {
                defer {acl_free(UnsafeMutableRawPointer(current))}
                guard acl_valid(current)==0 else {throw failure()}
                var entry:acl_entry_t?
                guard acl_get_entry(current,Int32(ACL_FIRST_ENTRY.rawValue),&entry) != 0 else {throw failure(EACCES)}
            } else if errno != ENOENT {throw failure()}
        }
        try verifyPrivate(staging,mode:0o700)
        let access=replace ? O_WRONLY:O_RDWR
        let file=openat(staging,"data",access|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0o600)
        guard file>=0 else {throw failure()}
        var committed=false
        defer {close(file);if !committed {unlinkat(staging,"data",0)}}
        guard fchmod(file,0o600)==0 else {throw failure()}
        try verifyPrivate(file,mode:0o600)
        try bytes.withUnsafeBytes {raw in
            var offset=0
            while offset<raw.count {
                let count=Darwin.write(file,raw.baseAddress!.advanced(by:offset),raw.count-offset)
                if count<0 && errno==EINTR {continue}
                guard count>0 else {throw failure(count==0 ? EIO:errno)}
                offset+=count
            }
        }
        if synchronize {guard fsync(file)==0 else {throw failure()}}
        let status=replace ? renameat(staging,"data",directory,url.lastPathComponent):renameatx_np(staging,"data",directory,url.lastPathComponent,UInt32(RENAME_EXCL))
        guard status==0 else {throw failure()}
        committed=true
        if synchronize,fsync(directory) != 0 {
            let code=errno
            // Create-only callers can retry after an unsuccessful publication.
            // Preserve replacement paths and edits to the published inode.
            if !replace {
                var held=stat(),current=stat()
                if fstat(file,&held)==0,held.st_size==bytes.count,held.st_nlink==1 {
                    var observed=Data(count:bytes.count)
                    let read=observed.withUnsafeMutableBytes {raw -> Bool in
                        var offset=0
                        while offset<raw.count {
                            let count=pread(file,raw.baseAddress!.advanced(by:offset),raw.count-offset,off_t(offset))
                            if count<0 && errno==EINTR {continue}
                            guard count>0 else {return false};offset+=count
                        }
                        return true
                    }
                    if read,observed==bytes,fstatat(directory,url.lastPathComponent,&current,AT_SYMLINK_NOFOLLOW)==0,
                       (current.st_mode & S_IFMT)==S_IFREG,current.st_uid==getuid(),current.st_nlink==1,
                       current.st_dev==held.st_dev,current.st_ino==held.st_ino {
                        unlinkat(directory,url.lastPathComponent,0)
                    }
                }
            }
            throw failure(code)
        }
    }
}
