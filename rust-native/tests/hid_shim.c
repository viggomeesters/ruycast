// Full disposable HID/Quartz boundary. No function below calls an OS hook,
// discovers real devices/apps/permissions or posts actual input.
#include "caps_shim.c"
#include <ApplicationServices/ApplicationServices.h>
#include <Carbon/Carbon.h>
#include <IOKit/hid/IOHIDManager.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct Obj {
    unsigned kind; bool valid, enabled, scheduled;
    IOHIDValueCallback input; IOHIDDeviceCallback remove; CGEventTapCallBack tap;
    void *context;
    uint64_t flags; int64_t key, repeat, tag; uint32_t type; bool down;
} Obj;
static Obj *hid_manager, *hid_tap;
static unsigned allocated, posts, envflags, manager_creates;
static uint64_t last_flags, last_event_flags; static bool last_down;
static char failure[100], hook_name[100];
static void (*hid_hook)(void *);static void *hid_hook_context;
static double now;
static int devices[16];
static bool quiet;
void ruycast_hid_fixture_quiet(bool value){quiet=value;}
static int hidcall(const char *name) {
    if(!quiet)fprintf(stderr,"hid-fixture:%s\n",name);
    if(hid_hook && strcmp(hook_name,name)==0){void (*callback)(void *)=hid_hook;hid_hook=NULL;callback(hid_hook_context);}
#ifdef RUYCAST_HID_HOST_FIXTURE
    const char *home=getenv("RUYCAST_KEYBOARD_FIXTURE_HOME");
    if(home){
        char path[4096];snprintf(path,sizeof(path),"%s/hid-hold-%s",home,name);
        if(access(path,F_OK)==0){
            char entered[4096];snprintf(entered,sizeof(entered),"%s/hid-entered-%s",home,name);
            FILE *w=fopen(entered,"w");assert(w);fputs("entered",w);fclose(w);
            for(unsigned i=0;i<5000 && access(path,F_OK)==0;i++)usleep(1000);
            assert(access(path,F_OK)!=0);
        }
        snprintf(path,sizeof(path),"%s/hid-fail-%s",home,name);if(access(path,F_OK)==0)return 1;
    }
#endif
    return strcmp(failure,name)==0;
}
static Obj *object(unsigned kind){Obj *value=calloc(1,sizeof(*value));assert(value);value->kind=kind;value->valid=true;allocated++;return value;}
uint64_t ruycast_hid_fixture_ready(void){
#ifdef RUYCAST_HID_HOST_FIXTURE
    extern uint64_t ruycast_keyboard_fixture_ready(void);
    static bool initialized;
    if(!ruycast_keyboard_fixture_ready())return 0;
    if(!initialized){latch=true;quiet=true;initialized=true;}
#endif
    return UINT64_C(0x5255484944303031);
}
void ruycast_hid_fixture_configure(bool original,unsigned environment){assert(!allocated);ruycast_caps_fixture_configure(original,0);posts=0;last_flags=0;last_down=false;envflags=environment;failure[0]=0;hid_hook=NULL;now=0;}
void ruycast_hid_fixture_failure(const char *name){snprintf(failure,sizeof(failure),"%s",name);}
void ruycast_hid_fixture_hook(const char *name,void(*callback)(void *),void *context){snprintf(hook_name,sizeof(hook_name),"%s",name);hid_hook=callback;hid_hook_context=context;}
void ruycast_hid_fixture_environment(unsigned value){envflags=value;}
const char *ruycast_hid_fixture_state(void){static char buffer[500];snprintf(buffer,sizeof(buffer),"{\"allocated\":%u,\"managerCreates\":%u,\"posts\":%u,\"lastFlags\":%llu,\"lastDown\":%s,\"caps\":%u,\"manager\":%s,\"tap\":%s}",allocated,manager_creates,posts,(unsigned long long)last_flags,last_down?"true":"false",ruycast_caps_fixture_state(),hid_manager?"true":"false",hid_tap?"true":"false");return buffer;}
IOHIDManagerRef ruycast_hid_fixture_IOHIDManagerCreate(CFAllocatorRef allocator,IOOptionBits options){assert(!allocator&&options==0);if(hidcall("manager_create"))return NULL;assert(!hid_manager);manager_creates++;hid_manager=object(1);return (IOHIDManagerRef)hid_manager;}
IOReturn ruycast_hid_fixture_IOHIDManagerOpen(IOHIDManagerRef manager,IOOptionBits options){assert((Obj *)manager==hid_manager&&options==0);return hidcall("manager_open")?-101:0;}
IOReturn ruycast_hid_fixture_IOHIDManagerClose(IOHIDManagerRef manager,IOOptionBits options){assert((Obj *)manager==hid_manager&&options==0);return hidcall("manager_close")?-102:0;}
void ruycast_hid_fixture_IOHIDManagerSetDeviceMatching(IOHIDManagerRef manager,CFDictionaryRef dict){assert((Obj *)manager==hid_manager&&((Obj *)dict)->kind==4);hidcall("device_matching");}
void ruycast_hid_fixture_IOHIDManagerSetInputValueMatching(IOHIDManagerRef manager,CFDictionaryRef dict){assert((Obj *)manager==hid_manager&&((Obj *)dict)->kind==5);hidcall("input_matching");}
void ruycast_hid_fixture_IOHIDManagerRegisterInputValueCallback(IOHIDManagerRef manager,IOHIDValueCallback callback,void *context){assert((Obj *)manager==hid_manager);hid_manager->input=callback;hid_manager->context=context;hidcall(callback?"register_input":"unregister_input");}
void ruycast_hid_fixture_IOHIDManagerRegisterDeviceRemovalCallback(IOHIDManagerRef manager,IOHIDDeviceCallback callback,void *context){assert((Obj *)manager==hid_manager);hid_manager->remove=callback;hid_manager->context=context;hidcall(callback?"register_remove":"unregister_remove");}
void ruycast_hid_fixture_IOHIDManagerScheduleWithRunLoop(IOHIDManagerRef manager,CFRunLoopRef loop,CFStringRef mode){assert((Obj *)manager==hid_manager&&(uintptr_t)loop==40&&(uintptr_t)mode==41);hid_manager->scheduled=true;hidcall("schedule");}
void ruycast_hid_fixture_IOHIDManagerUnscheduleFromRunLoop(IOHIDManagerRef manager,CFRunLoopRef loop,CFStringRef mode){assert((Obj *)manager==hid_manager&&(uintptr_t)loop==40&&(uintptr_t)mode==41);hid_manager->scheduled=false;hidcall("unschedule");}
typedef struct {uint32_t page,usage;void *device;bool down;} Input;
IOHIDElementRef ruycast_hid_fixture_IOHIDValueGetElement(IOHIDValueRef value){hidcall("element");return (IOHIDElementRef)value;}
CFIndex ruycast_hid_fixture_IOHIDValueGetIntegerValue(IOHIDValueRef value){hidcall("integer");return ((Input *)value)->down;}
uint32_t ruycast_hid_fixture_IOHIDElementGetUsagePage(IOHIDElementRef element){hidcall("usage_page");return ((Input *)element)->page;}
uint32_t ruycast_hid_fixture_IOHIDElementGetUsage(IOHIDElementRef element){hidcall("usage");return ((Input *)element)->usage;}
IOHIDDeviceRef ruycast_hid_fixture_IOHIDElementGetDevice(IOHIDElementRef element){hidcall("device");return (IOHIDDeviceRef)((Input *)element)->device;}
CFMachPortRef ruycast_hid_fixture_CGEventTapCreate(CGEventTapLocation location,CGEventTapPlacement place,CGEventTapOptions options,CGEventMask mask,CGEventTapCallBack callback,void *context){assert(location==kCGSessionEventTap&&place==kCGHeadInsertEventTap&&options==kCGEventTapOptionDefault&&mask==((UINT64_C(1)<<1)|(UINT64_C(1)<<3)|(UINT64_C(1)<<10)|(UINT64_C(1)<<11)|(UINT64_C(1)<<12)|(UINT64_C(1)<<25)|(UINT64_C(1)<<22)));if(hidcall("tap_create"))return NULL;assert(!hid_tap);hid_tap=object(2);hid_tap->tap=callback;hid_tap->context=context;return (CFMachPortRef)hid_tap;}
void ruycast_hid_fixture_CGEventTapEnable(CFMachPortRef port,bool enable){assert((Obj *)port==hid_tap);if(!hidcall(enable?"tap_enable":"tap_disable"))hid_tap->enabled=enable;}
bool ruycast_hid_fixture_CGEventTapIsEnabled(CFMachPortRef port){assert((Obj *)port==hid_tap);hidcall("tap_enabled");return hid_tap->enabled;}
Boolean ruycast_hid_fixture_CFMachPortIsValid(CFMachPortRef port){assert((Obj *)port==hid_tap);hidcall("port_valid");return hid_tap->valid;}
void ruycast_hid_fixture_CFMachPortInvalidate(CFMachPortRef port){assert((Obj *)port==hid_tap);if(!hidcall("invalidate"))hid_tap->valid=false;}
CFRunLoopSourceRef ruycast_hid_fixture_CFMachPortCreateRunLoopSource(CFAllocatorRef allocator,CFMachPortRef port,CFIndex order){assert(!allocator&&(Obj *)port==hid_tap&&order==0);return hidcall("source")?NULL:(CFRunLoopSourceRef)object(3);}
CFRunLoopRef ruycast_hid_fixture_CFRunLoopGetMain(void){hidcall("runloop");return (CFRunLoopRef)(uintptr_t)40;}
void ruycast_hid_fixture_CFRunLoopAddSource(CFRunLoopRef loop,CFRunLoopSourceRef source,CFRunLoopMode mode){assert((uintptr_t)loop==40&&((Obj *)source)->kind==3&&(uintptr_t)mode==41);hidcall("add_source");}
void ruycast_hid_fixture_CFRunLoopRemoveSource(CFRunLoopRef loop,CFRunLoopSourceRef source,CFRunLoopMode mode){assert((uintptr_t)loop==40&&((Obj *)source)->kind==3&&(uintptr_t)mode==41);hidcall("remove_source");}
void ruycast_hid_fixture_CFRelease(CFTypeRef ref){Obj *value=(Obj *)ref;assert(value&&allocated);hidcall("release");if(value==hid_tap){assert(!value->valid);hid_tap=NULL;}if(value==hid_manager){assert(!value->scheduled&&!value->input&&!value->remove);hid_manager=NULL;}allocated--;free(value);}
CGEventFlags ruycast_hid_fixture_CGEventGetFlags(CGEventRef event){hidcall("flags");return ((Obj *)event)->flags;}
void ruycast_hid_fixture_CGEventSetFlags(CGEventRef event,CGEventFlags flags){((Obj *)event)->flags=flags;hidcall("set_flags");}
int64_t ruycast_hid_fixture_CGEventGetIntegerValueField(CGEventRef event,CGEventField field){hidcall("field");Obj *value=(Obj *)event;if(field==kCGEventSourceUserData)return value->tag;if(field==kCGKeyboardEventKeycode)return value->key;assert(field==kCGKeyboardEventAutorepeat);return value->repeat;}
void ruycast_hid_fixture_CGEventSetIntegerValueField(CGEventRef event,CGEventField field,int64_t value){assert(field==kCGEventSourceUserData);((Obj *)event)->tag=value;hidcall("set_field");}
void ruycast_hid_fixture_CGEventSetType(CGEventRef event,CGEventType kind){((Obj *)event)->type=kind;hidcall("set_type");}
CGEventSourceRef ruycast_hid_fixture_CGEventSourceCreate(CGEventSourceStateID state){assert(state==kCGEventSourceStatePrivate);return hidcall("event_source")?NULL:(CGEventSourceRef)object(6);}
CGEventRef ruycast_hid_fixture_CGEventCreateKeyboardEvent(CGEventSourceRef source,CGKeyCode key,bool down){assert(((Obj *)source)->kind==6&&key==57);if(hidcall("keyboard_event"))return NULL;Obj *event=object(7);event->key=key;event->down=down;return (CGEventRef)event;}
#ifndef OMIT_HID_POST
void ruycast_hid_fixture_CGEventPost(CGEventTapLocation location,CGEventRef event){Obj *value=(Obj *)event;assert(location==kCGSessionEventTap&&value->type==kCGEventFlagsChanged&&value->tag==INT64_C(0x52554859504552));posts++;last_flags=value->flags;last_down=value->down;hidcall("post");}
#endif
Boolean ruycast_hid_fixture_IsSecureEventInputEnabled(void){hidcall("secure");return (envflags&1)!=0;}
Boolean ruycast_hid_fixture_AXIsProcessTrusted(void){hidcall("accessibility");return (envflags&2)==0;}
bool ruycast_hid_fixture_CGPreflightListenEventAccess(void){hidcall("monitoring");return (envflags&4)==0;}
void *ruycast_hid_fixture_dictionary(bool device){if(hidcall(device?"device_dictionary":"input_dictionary"))return NULL;return object(device?4:5);}
void *ruycast_hid_fixture_common(void){hidcall("common");return (void *)(uintptr_t)41;}
double ruycast_hid_fixture_uptime(void){hidcall("uptime");return now;}
bool ruycast_hid_fixture_raycast(void){hidcall("raycast");return (envflags&8)!=0;}
void ruycast_hid_fixture_input(uint32_t device,uint32_t page,uint32_t usage,bool down,double time){assert(device<16);now=time;Input input={page,usage,&devices[device],down};if(hid_manager&&hid_manager->scheduled&&hid_manager->input)hid_manager->input(hid_manager->context,0,hid_manager,(IOHIDValueRef)&input);}
void ruycast_hid_fixture_remove(uint32_t device){assert(device<16);if(hid_manager&&hid_manager->scheduled&&hid_manager->remove)hid_manager->remove(hid_manager->context,0,hid_manager,(IOHIDDeviceRef)&devices[device]);}
uint64_t ruycast_hid_fixture_event(uint32_t type,uint16_t key,uint64_t flags,int64_t tag,bool repeated){Obj value={0};value.key=key;value.flags=flags;value.tag=tag;value.repeat=repeated;CGEventRef result=(CGEventRef)&value;if(hid_tap&&hid_tap->enabled&&hid_tap->valid)result=hid_tap->tap((CGEventTapProxy)hid_tap,type,(CGEventRef)&value,hid_tap->context);last_event_flags=value.flags;
#ifdef RUYCAST_HID_HOST_FIXTURE
    if(result && type==kCGEventKeyDown && !repeated){
        extern OSStatus ruycast_keyboard_fixture_emit_binding(UInt32,UInt32);
        UInt32 modifiers=0;if(value.flags&kCGEventFlagMaskCommand)modifiers|=cmdKey;if(value.flags&kCGEventFlagMaskControl)modifiers|=controlKey;if(value.flags&kCGEventFlagMaskAlternate)modifiers|=optionKey;if(value.flags&kCGEventFlagMaskShift)modifiers|=shiftKey;
        ruycast_keyboard_fixture_emit_binding(key,modifiers);
    }
#endif
    return result==NULL;}

uint64_t ruycast_hid_fixture_event_flags(void){return last_event_flags;}

#define SDK_ABI(name) _Static_assert(__builtin_types_compatible_p(__typeof__(&(name)),__typeof__(&(ruycast_hid_fixture_##name))), #name " ABI")
SDK_ABI(IOHIDManagerCreate);
SDK_ABI(IOHIDManagerOpen);
SDK_ABI(IOHIDManagerClose);
SDK_ABI(IOHIDManagerSetDeviceMatching);
SDK_ABI(IOHIDManagerSetInputValueMatching);
SDK_ABI(IOHIDManagerRegisterInputValueCallback);
SDK_ABI(IOHIDManagerRegisterDeviceRemovalCallback);
SDK_ABI(IOHIDManagerScheduleWithRunLoop);
SDK_ABI(IOHIDManagerUnscheduleFromRunLoop);
SDK_ABI(IOHIDValueGetElement);
SDK_ABI(IOHIDValueGetIntegerValue);
SDK_ABI(IOHIDElementGetUsagePage);
SDK_ABI(IOHIDElementGetUsage);
SDK_ABI(IOHIDElementGetDevice);
SDK_ABI(CGEventTapCreate);
SDK_ABI(CGEventTapEnable);
SDK_ABI(CGEventTapIsEnabled);
SDK_ABI(CFMachPortIsValid);
SDK_ABI(CFMachPortInvalidate);
SDK_ABI(CFMachPortCreateRunLoopSource);
SDK_ABI(CFRunLoopGetMain);
SDK_ABI(CFRunLoopAddSource);
SDK_ABI(CFRunLoopRemoveSource);
SDK_ABI(CFRelease);
SDK_ABI(CGEventGetFlags);
SDK_ABI(CGEventSetFlags);
SDK_ABI(CGEventGetIntegerValueField);
SDK_ABI(CGEventSetIntegerValueField);
SDK_ABI(CGEventSetType);
SDK_ABI(CGEventSourceCreate);
SDK_ABI(CGEventCreateKeyboardEvent);
#ifndef OMIT_HID_POST
SDK_ABI(CGEventPost);
#endif
SDK_ABI(IsSecureEventInputEnabled);
SDK_ABI(AXIsProcessTrusted);
SDK_ABI(CGPreflightListenEventAccess);

_Static_assert(sizeof(Boolean)==1 && sizeof(CGEventMask)==8 && sizeof(IOOptionBits)==4 && sizeof(CFIndex)==sizeof(intptr_t),"Rust ABI scalars");
_Static_assert(kCGEventScrollWheel==22 && kCGEventOtherMouseDown==25 && kCGEventKeyDown==10 && kCGEventKeyUp==11 && kCGEventFlagsChanged==12 && kCGEventSourceUserData==42 && kCGKeyboardEventKeycode==9 && kCGKeyboardEventAutorepeat==8 && kCFNumberSInt32Type==3,"SDK constants");
