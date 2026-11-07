#include <vector>

int process(std::vector<int> &values)
{
    int total = 0;
    if (values.empty())
    {
        return 0;
    }
    for (int value : values)
    {
        total += value;
    }
    while (total > 100)
    {
        total -= 100;
    }
    return total;
}

int helper(int x)
{
    return x * 2;
}
