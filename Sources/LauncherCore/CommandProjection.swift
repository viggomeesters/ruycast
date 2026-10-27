import LauncherModels

/// Pure presentation ordering stays in the speed-optimized core. The native
/// host passes one value snapshot; no UI actor is consulted by the comparator.
public enum CommandProjection {
    public static func emptyQuery(_ commands:[Command],usage:[String:Int],limit:Int)->[Command] {
        if usage.isEmpty {return limit<=0 ? commands:Array(commands.prefix(limit))}
        let sorted=commands.enumerated().sorted {a,b in
            let x=usage[a.element.id] ?? 0,y=usage[b.element.id] ?? 0
            return x==y ? a.offset<b.offset:x>y
        }
        return Array(sorted.prefix(limit<=0 ? commands.count:limit).map(\.element))
    }
}
