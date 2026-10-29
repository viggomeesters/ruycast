import ApplicationServices
import Foundation

/// Bounded typed reads shared by app menus and open-window search.
enum SystemAccessibility {
    static func boolean(_ value:CFTypeRef?)->Bool? {
        guard let value,CFGetTypeID(value)==CFBooleanGetTypeID() else {return nil}
        return value as? Bool
    }
    static func integer(_ value:CFTypeRef?)->Int? {
        guard let value,CFGetTypeID(value)==CFNumberGetTypeID() else {return nil}
        return (value as? NSNumber)?.intValue
    }
    static func pid(_ element:AXUIElement)->pid_t? {
        var pid:pid_t=0
        return AXUIElementGetPid(element,&pid) == .success ? pid:nil
    }
    static func value(_ element:AXUIElement,_ key:String)->CFTypeRef? {
        AXUIElementSetMessagingTimeout(element,0.12)
        var result:CFTypeRef?
        guard AXUIElementCopyAttributeValue(element,key as CFString,&result) == .success else {return nil}
        return result
    }
    static func children(_ element:AXUIElement,_ limit:Int)->[AXUIElement]? {elements(element,kAXChildrenAttribute,limit,emptyWhenUnsupported:true)}
    static func elements(_ element:AXUIElement,_ key:String,_ limit:Int,emptyWhenUnsupported:Bool=false)->[AXUIElement]? {
        guard limit>0 else {return []}
        AXUIElementSetMessagingTimeout(element,0.12)
        var count:CFIndex=0,raw:CFArray?
        let status=AXUIElementGetAttributeValueCount(element,key as CFString,&count)
        if status == .attributeUnsupported {return emptyWhenUnsupported ? []:nil}
        guard status == .success else {return nil}
        if count==0 {return []}
        guard AXUIElementCopyAttributeValues(element,key as CFString,0,min(count,limit),&raw) == .success else {return nil}
        return raw as? [AXUIElement]
    }
}
