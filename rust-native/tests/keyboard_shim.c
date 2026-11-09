// Complete owned Carbon boundary. No real event target/handler/registration API
// is called; callbacks receive synthetic C structs on the process main thread.
#include <Carbon/Carbon.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
typedef struct {UInt32 event_class,kind,signature,id;OSStatus status;UInt32 parameter_type;ByteCount size;} FixtureEvent;
_Static_assert(sizeof(EventHotKeyID)==8,"Carbon ID ABI");
_Static_assert(sizeof(ByteCount)==sizeof(size_t),"Carbon ByteCount ABI");
_Static_assert(cmdKey==(1<<8)&&shiftKey==(1<<9)&&optionKey==(1<<11)&&controlKey==(1<<12),"Carbon modifier masks");
typedef struct {UInt32 code,modifiers,id;} Key;
static EventHandlerUPP callback;
static void *userdata;
static Key *keys[256];
static int target_token,handler_token;
static const char *home(void){return getenv("RUYCAST_KEYBOARD_FIXTURE_HOME");}
static bool flag(const char *name){char path[4096];if(!home())return false;snprintf(path,sizeof(path),"%s/%s",home(),name);return access(path,F_OK)==0;}
static void record(const char *phase,long a,long b,long c){
    if(!home()||!pthread_main_np())abort();char path[4096];snprintf(path,sizeof(path),"%s/keyboard-trace.jsonl",home());
    FILE *f=fopen(path,"a");if(!f)abort();fprintf(f,"{\"phase\":\"%s\",\"a\":%ld,\"b\":%ld,\"c\":%ld,\"mainThread\":true}\n",phase,a,b,c);fclose(f);
}
static int fail_code(void){char path[4096];snprintf(path,sizeof(path),"%s/fail-code",home());FILE *f=fopen(path,"r");int code=-1;if(f){if(fscanf(f,"%d",&code)!=1)code=-1;fclose(f);}return code;}
__attribute__((visibility("default"))) uint64_t ruycast_keyboard_fixture_ready(void){
    if(!home())return 0;char path[4096],text[64]={0};snprintf(path,sizeof(path),"%s/.ruycast-keyboard-fixture",home());FILE *f=fopen(path,"r");if(!f)return 0;size_t n=fread(text,1,sizeof(text)-1,f);fclose(f);(void)n;
    return strcmp(text,"own-keyboard-fixture-v1")==0?UINT64_C(0x52554b4559533031):0;
}
__attribute__((visibility("default"))) EventTargetRef ruycast_keyboard_fixture_target(void){record("target",0,0,0);return (EventTargetRef)&target_token;}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_install(EventTargetRef target,EventHandlerUPP handler,UInt32 count,const EventTypeSpec *types,void *data,EventHandlerRef *out){
    if(target!=(EventTargetRef)&target_token||count!=1||types->eventClass!=kEventClassKeyboard||types->eventKind!=kEventHotKeyPressed||!handler||!data)abort();record("install",count,types->eventClass,types->eventKind);
    if(flag("fail-install"))return -50;callback=handler;userdata=data;*out=flag("null-install")?NULL:(EventHandlerRef)&handler_token;return 0;
}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_remove(EventHandlerRef handler){
    if(handler!=(EventHandlerRef)&handler_token)abort();record("remove",0,0,0);if(flag("fail-remove"))return -50;callback=NULL;userdata=NULL;return 0;
}
__attribute__((visibility("default"))) UInt32 ruycast_keyboard_fixture_class(EventRef event){return ((FixtureEvent *)event)->event_class;}
__attribute__((visibility("default"))) UInt32 ruycast_keyboard_fixture_kind(EventRef event){return ((FixtureEvent *)event)->kind;}
#ifndef OMIT_PARAMETER
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_parameter(EventRef event,EventParamName name,EventParamType type,EventParamType *actual,ByteCount capacity,ByteCount *size,void *out){
    if(name!=kEventParamDirectObject||type!=typeEventHotKeyID||capacity!=sizeof(EventHotKeyID)||!actual||!size||!out)abort();FixtureEvent *e=(FixtureEvent *)event;record("parameter",e->id,capacity,e->size);*actual=e->parameter_type;*size=e->size;
    EventHotKeyID id={e->signature,e->id};memcpy(out,&id,sizeof(id));return e->status;
}
#endif
static void reenter(UInt32 id,const char *phase){
    FixtureEvent e={kEventClassKeyboard,kEventHotKeyPressed,0x52554341,id,0,typeEventHotKeyID,sizeof(EventHotKeyID)};
    OSStatus status=callback?callback(NULL,(EventRef)&e,userdata):eventNotHandledErr;record(phase,id,status,0);
}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_register(UInt32 code,UInt32 modifiers,EventHotKeyID id,EventTargetRef target,OptionBits options,EventHotKeyRef *out){
    if(target!=(EventTargetRef)&target_token||options!=0||id.signature!=0x52554341||!out)abort();record("register",code,modifiers,id.id);
    if(flag("hold-register")){
        char entered[4096],release[4096];snprintf(entered,sizeof(entered),"%s/keyboard-register-entered",home());snprintf(release,sizeof(release),"%s/keyboard-register-release",home());
        FILE *f=fopen(entered,"w");if(!f)abort();fputs("entered",f);fclose(f);
        unsigned attempts=0;while(access(release,F_OK)!=0){if(++attempts>5000)return -9878;if(flag("pump-register"))CFRunLoopRunInMode(kCFRunLoopDefaultMode,.001,true);else usleep(1000);}
    }
    if((int)code==fail_code())return -9878;
    for(size_t i=0;i<256;i++){if(keys[i]&&keys[i]->code==code&&keys[i]->modifiers==modifiers)return -9878;}
    for(size_t i=0;i<256;i++){if(!keys[i]){Key *key=malloc(sizeof(*key));if(!key)abort();*key=(Key){code,modifiers,id.id};keys[i]=key;*out=(EventHotKeyRef)key;if(flag("emit-on-register"))reenter(id.id,"register-callback");return flag("partial-register")?-50:0;}}
    return -108;
}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_unregister(EventHotKeyRef handle){
    if(flag("emit-latest-on-unregister")){
        UInt32 latest=0;for(size_t i=0;i<256;i++)if(keys[i]&&keys[i]->id>latest)latest=keys[i]->id;
        if(latest)reenter(latest,"unregister-latest-callback");
    }
    if(flag("hold-unregister")){
        char entered[4096],release[4096];snprintf(entered,sizeof(entered),"%s/keyboard-unregister-entered",home());snprintf(release,sizeof(release),"%s/keyboard-unregister-release",home());
        FILE *f=fopen(entered,"w");if(!f)abort();fputs("entered",f);fclose(f);
        unsigned attempts=0;while(access(release,F_OK)!=0){if(++attempts>5000)return -50;CFRunLoopRunInMode(kCFRunLoopDefaultMode,.001,true);}
    }
    for(size_t i=0;i<256;i++){if((void *)keys[i]==(void *)handle&&keys[i]){record("unregister",keys[i]->code,keys[i]->modifiers,keys[i]->id);if(flag("emit-on-unregister"))reenter(keys[i]->id,"unregister-callback");if(flag("fail-unregister"))return -50;free(keys[i]);keys[i]=NULL;return 0;}}
    abort();
}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_emit(const FixtureEvent *event){return callback?callback(NULL,(EventRef)event,userdata):eventNotHandledErr;}
__attribute__((destructor)) static void cleanup(void){for(size_t i=0;i<256;i++){free(keys[i]);keys[i]=NULL;}}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_emit_binding(UInt32 code,UInt32 modifiers){
    for(size_t i=0;i<256;i++)if(keys[i]&&keys[i]->code==code&&keys[i]->modifiers==modifiers){UInt32 id=keys[i]->id;FixtureEvent e={kEventClassKeyboard,kEventHotKeyPressed,0x52554341,id,0,typeEventHotKeyID,sizeof(EventHotKeyID)};OSStatus status=callback?callback(NULL,(EventRef)&e,userdata):eventNotHandledErr;record("emit-binding",id,status,0);return status;}
    record("emit-missing",code,modifiers,0);return eventNotHandledErr;
}
__attribute__((visibility("default"))) OSStatus ruycast_keyboard_fixture_emit_id(UInt32 id){
    FixtureEvent e={kEventClassKeyboard,kEventHotKeyPressed,0x52554341,id,0,typeEventHotKeyID,sizeof(EventHotKeyID)};OSStatus status=callback?callback(NULL,(EventRef)&e,userdata):eventNotHandledErr;record("emit-id",id,status,0);return status;
}
