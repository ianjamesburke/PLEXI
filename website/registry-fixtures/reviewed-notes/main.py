from plexi_sdk import Badge, Column, Text


def view():
    return Column(
        Text("Reviewed Notes"),
        Badge("free hosted registry smoke"),
        Text("This app is distributed as a reviewed native package."),
    )
