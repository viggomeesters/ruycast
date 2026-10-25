import AppKit
import LauncherCore

/// Delayed delivery reproduces cancel/reopen of the same ID without desktop I/O.
func checkClipboardPreviewCancellation(_ store:ClipboardStore)throws->[String:Any] {
    guard let context=CGContext(data:nil,width:1,height:1,bitsPerComponent:8,bytesPerRow:4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue),let image=context.makeImage() else {throw LauncherError("No preview fixture")}
    let lock=NSLock();var callbacks:[()->Void]=[]
    let preview=ClipboardPreviewModel(decode:{clip,_ in ClipboardImage(image:image,width:clip.text=="old" ? 1:2,height:1,bytes:4)},deliver:{callback in lock.lock();callbacks.append(callback);lock.unlock()})
    func count()->Int {lock.lock();defer {lock.unlock()};return callbacks.count}
    func callback(_ index:Int)->()->Void {lock.lock();defer {lock.unlock()};return callbacks[index]}
    func wait(_ expected:Int)throws {
        let end=Date().addingTimeInterval(3)
        while count()<expected && Date()<end {Thread.sleep(forTimeInterval:0.002)}
        guard count()>=expected else {throw LauncherError("Preview delivery fixture timed out")}
    }
    var old=Clip(text:"old",source:"fixture",contentType:"image"),new=Clip(text:"new",source:"fixture",contentType:"image");new.id=old.id
    preview.load(old,store:store);try wait(1)
    preview.load(nil,store:store);preview.load(new,store:store);try wait(2)
    callback(0)()
    guard preview.loading,preview.asset==nil else {throw LauncherError("Cancelled delivery overwrote reopened preview")}
    callback(1)()
    guard !preview.loading,preview.asset?.width==2 else {throw LauncherError("Latest preview failed to publish")}
    old.id="deleted-fixture";preview.load(old,store:store);try wait(3)
    preview.load(nil,store:store);callback(2)()
    guard !preview.loading,preview.asset==nil,preview.error==nil else {throw LauncherError("Deleted preview returned after cancellation")}
    return ["sameIDReopenRejectsStaleDelivery":true,"deletionRejectsQueuedDelivery":true,"decodedAssetReleasedOnCancellation":true,"desktopEffects":false]
}
