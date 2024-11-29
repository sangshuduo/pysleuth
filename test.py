def get_value(my_dict):
    return my_dict['key']

def caller_function():
    value = get_value({'another_key': 'value'})

def higher_level_function():
    try:
        caller_function()
    except KeyError:
        print("Handled KeyError")

def unhandled_function():
    caller_function()

get_value({'key': 'value'})  # Safe direct call

try:
    get_value({'another_key': 'value'})  # Handled KeyError
except KeyError:
    print("Handled KeyError in main")

unhandled_function()  # Should produce a warning
