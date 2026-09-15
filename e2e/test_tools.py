async def test_component_exposes_its_tools(client):
    """A component with no tools is almost always a packaging mistake."""
    tools = await client.list_tools()
    assert len(tools) >= 1
