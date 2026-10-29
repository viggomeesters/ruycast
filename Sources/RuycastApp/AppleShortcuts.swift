import Foundation
import Darwin
import LauncherModels

/// Discovery is on demand. Names are display data; execution always uses the UUID.
enum AppleShortcuts {
    static let prefix="apple-shortcut."
    static let failure="Shortcut did not finish. Check Shortcuts before retrying."
    static let timeoutFailure="Shortcut run timed out. Its result is unknown. Check Shortcuts before retrying."
    static func identifier(_ value:String)->String? {
        guard value.count==36,let uuid=UUID(uuidString:value),uuid.uuidString.lowercased()==value.lowercased() else {return nil}
        return uuid.uuidString.lowercased()
    }
    static func recognizes(_ command:Command)->Bool {
        guard command.id.hasPrefix(prefix),let id=identifier(String(command.id.dropFirst(prefix.count))),command.id==prefix+id else {return false}
        return command.kind=="script" && command.target=="/usr/bin/shortcuts" && command.arguments==["run",id]
    }
    static func response(_ runner:[String:Any])->[String:Any] {
        // Only the trusted runner's status is interpreted. Workflow output is private data.
        if runner["ok"] as? Bool==true {return ["ok":true,"result":["state":"completed"]]}
        let error=runner["error"] as? String ?? "",prefix="Script timed out after ",suffix=" seconds."
        let boundedTimeout=error.hasPrefix(prefix) && error.hasSuffix(suffix) && Double(error.dropFirst(prefix.count).dropLast(suffix.count)) != nil
        return ["ok":false,"error":boundedTimeout ? timeoutFailure:failure]
    }
    static func parse(_ data:Data)throws->[Command] {
        guard let text=String(data:data,encoding:.utf8) else {throw LauncherError("Could not read shortcuts. Refresh to retry.")}
        var commands=[Command](),seen=Set<String>()
        for raw in text.split(separator:"\n",omittingEmptySubsequences:false) {
            let line=String(raw).trimmingCharacters(in:.whitespaces)
            if line.isEmpty {continue}
            guard line.count>=39,line.hasSuffix(")") else {throw LauncherError("Could not read shortcut names. Check Shortcuts, then refresh.")}
            let start=line.index(line.endIndex,offsetBy:-38)
            guard line[start]=="(",start>line.startIndex,line[line.index(before:start)].isWhitespace,
                  let id=identifier(String(line[line.index(after:start)..<line.index(before:line.endIndex)])),seen.insert(id).inserted else {throw LauncherError("Could not read shortcut identifiers. Refresh to retry.")}
            let name=line[..<start].split(whereSeparator:{$0.isWhitespace}).joined(separator:" ")
            guard !name.isEmpty,!name.unicodeScalars.contains(where:CharacterSet.controlCharacters.contains) else {throw LauncherError("Could not read shortcut names. Check Shortcuts, then refresh.")}
            guard commands.count<1000 else {throw LauncherError("Too many shortcuts to search. The limit is 1,000.")}
            commands.append(Command(id:prefix+id,title:name,kind:"script",target:"/usr/bin/shortcuts",arguments:["run",id],timeout:300))
        }
        return commands
    }
    static func discover(_ ticket:SystemSearchSession.Ticket)throws->[Command] {
        try parse(list(ticket))
    }
    /// Both pipes and the owned CLI child share a monotonic deadline. No read-to-EOF wait.
    /// Terminating this client never claims cancellation of a Shortcuts daemon/workflow.
    static func list(_ ticket:SystemSearchSession.Ticket,executable:String="/usr/bin/shortcuts",arguments:[String]=["list","--show-identifiers"],timeout:TimeInterval=8,byteLimit:Int=65_536)throws->Data {
        let process=Process(),out=Pipe(),err=Pipe()
        process.executableURL=URL(fileURLWithPath:executable);process.arguments=arguments
        process.standardInput=FileHandle.nullDevice;process.standardOutput=out;process.standardError=err
        let handles=[out.fileHandleForReading,err.fileHandleForReading]
        for handle in handles {
            let flags=fcntl(handle.fileDescriptor,F_GETFL)
            guard flags>=0,fcntl(handle.fileDescriptor,F_SETFL,flags|O_NONBLOCK)>=0 else {throw LauncherError("Could not load shortcuts. Refresh to retry.")}
        }
        defer {for handle in handles {try? handle.close()};try? out.fileHandleForWriting.close();try? err.fileHandleForWriting.close()}
        do {guard try ticket.launch(process) else {throw LauncherError("Shortcut search cancelled.")}}
        catch {throw LauncherError("Could not load shortcuts. Check Shortcuts, then refresh.")}
        try? out.fileHandleForWriting.close();try? err.fileHandleForWriting.close()
        defer {
            if process.isRunning {
                process.terminate()
                let grace=ProcessInfo.processInfo.systemUptime+0.15
                while process.isRunning,ProcessInfo.processInfo.systemUptime<grace {Thread.sleep(forTimeInterval:0.005)}
                if process.isRunning {
                    _=Darwin.kill(process.processIdentifier,SIGKILL)
                    let reapDeadline=ProcessInfo.processInfo.systemUptime+0.15
                    while process.isRunning,ProcessInfo.processInfo.systemUptime<reapDeadline {Thread.sleep(forTimeInterval:0.005)}
                }
            }
            // Foundation's Process termination handler reaps the direct child; never wait on inherited pipes.
            ticket.detach(process)
        }
        var data=[Data(),Data()],eof=[false,false],buffer=[UInt8](repeating:0,count:8192)
        let deadline=ProcessInfo.processInfo.systemUptime+timeout
        while true {
            guard ticket.current else {throw LauncherError("Shortcut search cancelled.")}
            guard ProcessInfo.processInfo.systemUptime<deadline else {throw LauncherError("Loading shortcuts timed out. Refresh to retry.")}
            for index in 0..<2 where !eof[index] {
                let size=Darwin.read(handles[index].fileDescriptor,&buffer,buffer.count)
                if size>0 {
                    guard data[index].count+size<=byteLimit else {throw LauncherError("Shortcut list is too large to load.")}
                    data[index].append(contentsOf:buffer.prefix(size))
                } else if size==0 {eof[index]=true}
                else if errno != EAGAIN && errno != EINTR {throw LauncherError("Could not load shortcuts. Refresh to retry.")}
            }
            if !process.isRunning,eof.allSatisfy({$0}) {break}
            Thread.sleep(forTimeInterval:0.002)
        }
        guard process.terminationStatus==0 else {throw LauncherError("Could not load shortcuts. Check Shortcuts, then refresh.")}
        return data[0]
    }
}
