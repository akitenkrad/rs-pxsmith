extends Node2D


func _ready() -> void:
    var args := OS.get_cmdline_user_args()
    var capture_index := args.find("--capture")
    if capture_index < 0:
        return
    if capture_index + 1 >= args.size():
        push_error("--capture requires an output path")
        get_tree().quit(2)
        return

    await get_tree().process_frame
    await get_tree().process_frame
    await get_tree().process_frame
    var error := get_viewport().get_texture().get_image().save_png(args[capture_index + 1])
    if error != OK:
        push_error("Could not save capture: %s" % error_string(error))
        get_tree().quit(1)
        return
    get_tree().quit()
